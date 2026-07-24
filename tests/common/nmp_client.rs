//! Test relay actor backed by the same supported NMP facade as production.

#![allow(dead_code)]

use anyhow::{Context, Result};
use nmp::{
    AccessContext, AccountRegistration, AuthPolicy, AuthPolicyOp, AuthPolicyRegistration,
    AuthPolicyRequest, Engine, EngineConfig, RelayUrl, Window, WriteStatus,
};
use nmp_grammar::{Durability, HostAuthority, WriteIntent, WritePayload, WriteRouting};
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
pub struct WriteOutcome {
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
        if let Ok(url) = url::Url::parse(relay.as_str()) {
            if let Some(host) = url.host_str() {
                if !nmp::admits_network_relay_hint(&relay) {
                    config.allowed_local_relay_hosts.push(host.to_string());
                }
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

    pub async fn send_event_builder(&self, builder: EventBuilder) -> Result<WriteOutcome> {
        let event = builder
            .sign_with_keys(&self.keys)
            .context("sign test event")?;
        self.send_event(&event).await
    }

    pub async fn send_event(&self, event: &Event) -> Result<WriteOutcome> {
        let receiver = self
            .engine
            .publish(WriteIntent {
                payload: WritePayload::Signed(event.clone()),
                durability: Durability::Durable,
                routing: WriteRouting::PinnedHost(HostAuthority::from_selected_host(
                    self.relay.clone(),
                )),
                identity_override: Some(event.pubkey),
            })
            .context("submit NMP test write")?;
        let relay = self.relay.clone();
        let event_id = event.id;
        tokio::task::spawn_blocking(move || wait_for_write(receiver, relay, event_id))
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

fn wait_for_write(
    receiver: std::sync::mpsc::Receiver<WriteStatus>,
    relay: RelayUrl,
    event_id: EventId,
) -> Result<WriteOutcome> {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            anyhow::bail!("timed out waiting for NMP test write receipt");
        }
        match receiver.recv_timeout(remaining) {
            Ok(WriteStatus::Acked(acked)) => {
                return Ok(WriteOutcome {
                    val: event_id,
                    success: BTreeSet::from([acked.to_string()]),
                    failed: BTreeMap::new(),
                })
            }
            Ok(WriteStatus::Rejected(rejected, reason)) => {
                return Ok(WriteOutcome {
                    val: event_id,
                    success: BTreeSet::new(),
                    failed: BTreeMap::from([(rejected.to_string(), reason)]),
                })
            }
            Ok(WriteStatus::GaveUp(failed)) => {
                return Ok(WriteOutcome {
                    val: event_id,
                    success: BTreeSet::new(),
                    failed: BTreeMap::from([(
                        failed.to_string(),
                        "NMP gave up delivery".to_string(),
                    )]),
                })
            }
            Ok(WriteStatus::Failed(reason)) => anyhow::bail!("NMP test write failed: {reason}"),
            Ok(_) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                anyhow::bail!("timed out waiting for NMP test write receipt")
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("NMP test write receipt disconnected for {relay}")
            }
        }
    }
}
