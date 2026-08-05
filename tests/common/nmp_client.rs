//! Test relay actor backed by the same supported NMP facade as production.

#![allow(dead_code)]

use anyhow::{Context, Result};
use nmp::{
    AccessContext, AccountRegistration, AuthPolicy, AuthPolicyOp, AuthPolicyRegistration,
    AuthPolicyRequest, Engine, EngineConfig, FifoReceiver, FifoRecvTimeoutError, NotSentReason,
    RelayState, RelayUrl, SigningState, Window, WriteFact, WriteOutcome,
};
use nmp_grammar::{Identity, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventBuilder, EventId, Filter, Keys};
use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroUsize,
    time::{Duration, Instant},
};

#[path = "nmp_client/read.rs"]
mod read;
use read::{nmp_filter, pinned_query, receive_window};

pub struct NmpRelayClient {
    engine: Engine,
    relay: RelayUrl,
    keys: Keys,
    _accounts: Vec<AccountRegistration>,
    _auth_policies: Vec<AuthPolicyRegistration>,
}

#[derive(Debug)]
pub struct WriteAck {
    pub val: EventId,
    pub success: BTreeSet<String>,
    pub failed: BTreeMap<String, String>,
}

#[derive(Clone)]
struct AllowExactRelay {
    pubkey: nostr::PublicKey,
    relay: RelayUrl,
}

impl AuthPolicy for AllowExactRelay {
    fn evaluate(&self, request: AuthPolicyRequest) -> AuthPolicyOp {
        if request.expected_pubkey() == self.pubkey && request.relay() == &self.relay {
            AuthPolicyOp::allow()
        } else {
            AuthPolicyOp::deny("test client AUTH identity or relay mismatch")
        }
    }
}

impl NmpRelayClient {
    pub async fn connect(keys: Keys, relay: &str) -> Result<Self> {
        let relay = RelayUrl::parse(relay).context("parse test relay URL")?;
        let mut config = EngineConfig {
            app_relays: vec![relay.to_string()],
            ..EngineConfig::default()
        };
        if let Some(host) = nmp_grammar::relay::relay_host_key(&relay) {
            if nmp_network_policy::classify_bare_host(&host) == nmp_network_policy::HostClass::Local
                && !host.ends_with(".onion")
            {
                config.allowed_local_relay_hosts.push(host);
            }
        }
        let engine = Engine::new(config).context("start NMP test client")?;
        let account = engine
            .add_account(&keys.secret_key().to_secret_hex())
            .context("register NMP test account")?;
        let auth_policy = engine
            .add_auth_policy(
                keys.public_key(),
                AllowExactRelay {
                    pubkey: keys.public_key(),
                    relay: relay.clone(),
                },
            )
            .context("register NMP test AUTH policy")?;
        Ok(Self {
            engine,
            relay,
            keys,
            _accounts: vec![account],
            _auth_policies: vec![auth_policy],
        })
    }

    pub fn register_identity(&mut self, keys: &Keys) -> Result<()> {
        let account = self
            .engine
            .add_account(&keys.secret_key().to_secret_hex())
            .context("register additional NMP test account")?;
        let auth_policy = self
            .engine
            .add_auth_policy(
                keys.public_key(),
                AllowExactRelay {
                    pubkey: keys.public_key(),
                    relay: self.relay.clone(),
                },
            )
            .context("register additional NMP test AUTH policy")?;
        self._accounts.push(account);
        self._auth_policies.push(auth_policy);
        Ok(())
    }

    pub async fn send_event_builder(&self, builder: EventBuilder) -> Result<WriteAck> {
        let event = builder
            .sign_with_keys(&self.keys)
            .context("sign test event")?;
        self.send_event(&event).await
    }

    pub async fn send_event(&self, event: &Event) -> Result<WriteAck> {
        let receipt = self
            .engine
            .publish(WriteIntent {
                payload: WritePayload::Signed(event.clone()),
                routing: WriteRouting::Explicit(vec![self.relay.clone()]),
                identity: Identity::Explicit(event.pubkey),
                correlation: None,
            })
            .context("submit NMP test write")?;
        let relay = self.relay.clone();
        let event_id = event.id;
        tokio::task::spawn_blocking(move || wait_for_write(receipt.statuses, relay, event_id))
            .await
            .context("join NMP test write")?
    }

    pub async fn fetch_events(&self, filter: Filter, timeout: Duration) -> Result<Vec<Event>> {
        let max_rows = filter.limit.unwrap_or(200).max(1);
        let mut filter = nmp_filter(filter)?;
        filter.limit = None;
        let query = pinned_query(self.relay.clone(), filter, AccessContext::Public)?;
        let bound = NonZeroUsize::new(max_rows).expect("positive test read bound");
        let subscription = self
            .engine
            .observe(
                query,
                Some(Window::Expandable {
                    initial: bound,
                    max: bound,
                }),
            )
            .context("open NMP test read")?;
        tokio::task::spawn_blocking(move || receive_window(subscription, timeout))
            .await
            .context("join NMP test read")?
    }

    pub fn observe(&self, filter: Filter, access: AccessContext) -> Result<nmp::Subscription> {
        let query = pinned_query(self.relay.clone(), nmp_filter(filter)?, access)?;
        self.engine
            .observe(query, None)
            .context("open NMP test observation")
    }

    pub async fn disconnect(&self) {
        self.engine.shutdown();
    }
}

/// Drain one write's facts to its settlement.
///
/// This is a TEST HARNESS driving a real relay, not an app: proving what a
/// relay did with an event is the whole point of it. It is still allowed to
/// end only on a fact -- every stream carries exactly one `WriteOutcome` --
/// so the deadline below is a harness guard against a write that legitimately
/// parks forever (no signer, no resolvable route), never the way a settled
/// write is recognised.
fn wait_for_write(
    receiver: FifoReceiver<WriteFact>,
    relay: RelayUrl,
    event_id: EventId,
) -> Result<WriteAck> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut success = BTreeSet::new();
    let mut failed = BTreeMap::new();
    let mut last_fact = String::from("no fact observed");
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("NMP test write parked without settling (last fact: {last_fact})");
        }
        match receiver.recv_timeout(remaining) {
            Ok(WriteFact::Relay { relay, state }) => {
                last_fact = format!("{relay}: {state:?}");
                match state {
                    RelayState::Published => {
                        success.insert(relay.to_string());
                    }
                    RelayState::Rejected { reason } => {
                        failed.insert(relay.to_string(), reason);
                    }
                    // Never folded into a rejection: the app's own policy or
                    // signer declining is not the relay refusing the event.
                    RelayState::AuthFailed {
                        pubkey,
                        source,
                        reason,
                    } => {
                        failed.insert(
                            relay.to_string(),
                            format!("AUTH failed for {pubkey} ({source:?}): {reason}"),
                        );
                    }
                    RelayState::GaveUp => {
                        failed.insert(
                            relay.to_string(),
                            "NMP reached its publish attempt ceiling".to_string(),
                        );
                    }
                    RelayState::Waiting(_) | RelayState::Sent { .. } => {}
                }
            }
            Ok(WriteFact::Signing(SigningState::Refused { reason })) => {
                anyhow::bail!("NMP test write was refused by the signer: {reason}")
            }
            Ok(WriteFact::Signing(signing)) => last_fact = format!("{signing:?}"),
            Ok(WriteFact::Destinations {
                relays,
                complete,
                awaiting_author_routes,
            }) => {
                // `awaiting_author_routes` is WHY resolution is still open,
                // as keys. A probe that dropped it would report "still
                // resolving" with no way to say who it is waiting for.
                last_fact = format!(
                    "destinations {relays:?} complete={complete} awaiting={awaiting_author_routes:?}"
                );
            }
            Ok(WriteFact::Outcome(WriteOutcome::Settled)) => {
                return Ok(WriteAck {
                    val: event_id,
                    success,
                    failed,
                })
            }
            Ok(WriteFact::Outcome(WriteOutcome::NoDestination)) => {
                anyhow::bail!("NMP test write routing named no relays")
            }
            Ok(WriteFact::Outcome(WriteOutcome::NotSent(NotSentReason::Cancelled))) => {
                anyhow::bail!("NMP test write was cancelled")
            }
            Ok(WriteFact::Outcome(WriteOutcome::NotSent(NotSentReason::Superseded))) => {
                anyhow::bail!("NMP test write was superseded by a newer write")
            }
            Ok(WriteFact::Outcome(WriteOutcome::Refused(reason))) => {
                anyhow::bail!("NMP test write was refused at acceptance: {reason:?}")
            }
            Err(FifoRecvTimeoutError::Timeout) => {
                anyhow::bail!("NMP test write parked without settling (last fact: {last_fact})")
            }
            Err(FifoRecvTimeoutError::Closed) => {
                anyhow::bail!("NMP test write receipt disconnected for {relay}")
            }
            Err(FifoRecvTimeoutError::Lagged) => {
                anyhow::bail!("NMP test write receipt lagged for {relay}")
            }
        }
    }
}
