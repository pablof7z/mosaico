//! `Nip29Provider` — concrete NIP-29 wire, product decoding, and lifecycle boundary.

pub(crate) mod chat;
mod decoding;
mod group_management;
mod group_state;
mod group_topology;
mod membership_confirmation;
mod profiles;
mod publish_gate;
mod reactions;
mod readiness;

pub(crate) use publish_gate::ConfirmedGroupScope;

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::ChannelReadiness;
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::fava_host::FavaHost;
use crate::state::Store;
use anyhow::Result;
use std::sync::{Arc, Mutex};

// Fabric identifier used in all canonical origin rows.
pub const FABRIC: &str = "nip29";

/// Shell trait documenting the provider API surface.
#[allow(dead_code)]
pub trait FabricProvider {
    fn name(&self) -> &'static str;
}

/// Concrete provider for NIP-29 groups over Nostr events.
///
/// Fields held at construction time are stable config. Per-call dynamic data
/// (hosted "me" set, owners, now) is received as method parameters.
pub struct Nip29Provider {
    pub wire: Nip29WireCodec,
    /// Shared store Arc — same handle as `DaemonState.store`. No new Connection.
    pub store: Arc<Mutex<Store>>,
    /// Fava owns every relay read and write, signer selection, routing, and receipt.
    pub(crate) fava: Arc<FavaHost>,
    /// Human operator key (`userNsec`) for self-granting the management key.
    pub user_nsec: Option<String>,
    /// Whitelisted human pubkeys (hex) that should hold admin in owned groups.
    pub whitelisted_pubkeys: Vec<String>,
    /// TTL'd in-process cache of which channels are known-ready.
    pub readiness: Arc<ChannelReadiness>,
}

impl Nip29Provider {
    pub(crate) fn new(
        fava: Arc<FavaHost>,
        store: Arc<Mutex<Store>>,
        user_nsec: Option<String>,
        whitelisted_pubkeys: Vec<String>,
    ) -> Self {
        let wire = Nip29WireCodec;
        Self {
            wire,
            store,
            fava,
            user_nsec,
            whitelisted_pubkeys,
            readiness: Arc::new(ChannelReadiness::default()),
        }
    }

    pub fn name(&self) -> &'static str {
        "nip29"
    }

    /// Encode a domain event to an `EventBuilder` via the NIP-29 wire codec.
    pub fn encode(&self, ev: &DomainEvent) -> Result<nostr::event::EventBuilder> {
        self.wire.encode(ev)
    }

    /// Decode a raw envelope to a domain event via the NIP-29 wire codec.
    pub fn decode(&self, env: &RawEnvelope) -> Option<DomainEvent> {
        self.wire.decode(env)
    }

    /// Encode, sign, and durably enqueue one domain event. Relay delivery is
    /// always owned by Fava after this local acceptance boundary.
    pub async fn enqueue(
        &self,
        ev: &DomainEvent,
        keys: &nostr::key::Keys,
    ) -> Result<nostr::event::EventId> {
        match ev {
            DomainEvent::Status(status) => self.enqueue_status(status, keys, true, None).await,
            _ => {
                self.enqueue_with_confirmed_scope(ev, keys, None, None)
                    .await
            }
        }
    }

    /// Enqueue one status immediately after Fava returned a terminal group
    /// mutation result for one of its channels. Other channels still require
    /// current retained-observation evidence.
    pub(crate) async fn enqueue_status_after_confirmed_scope(
        &self,
        status: &crate::domain::Status,
        keys: &nostr::key::Keys,
        scope: &ConfirmedGroupScope,
        require_member: bool,
    ) -> Result<nostr::event::EventId> {
        self.enqueue_status(status, keys, require_member, Some(scope))
            .await
    }

    /// Publish a status with its lifecycle admission semantics made explicit.
    ///
    /// An offline status is a still-valid NIP-40 lease: its expiration says
    /// when observers may discard it, not whether its publisher was still a
    /// group member when it was emitted. Inferring membership from that
    /// timestamp breaks the post-leave expiry path under Fava, which correctly
    /// refuses already-expired events before they reach a relay.
    pub(crate) async fn enqueue_status(
        &self,
        status: &crate::domain::Status,
        keys: &nostr::key::Keys,
        require_member: bool,
        confirmed_scope: Option<&ConfirmedGroupScope>,
    ) -> Result<nostr::event::EventId> {
        self.enqueue_with_confirmed_scope(
            &DomainEvent::Status(status.clone()),
            keys,
            confirmed_scope,
            Some(require_member),
        )
        .await
    }

    async fn enqueue_with_confirmed_scope(
        &self,
        ev: &DomainEvent,
        keys: &nostr::key::Keys,
        confirmed_scope: Option<&ConfirmedGroupScope>,
        status_requires_membership: Option<bool>,
    ) -> Result<nostr::event::EventId> {
        // kind:0 profiles route to BOTH the indexer relay (purplepag.es) AND
        // the main NIP-29 relay(s) — the group relay accepts kind:0 fine, so
        // relying on the indexer alone leaves backend/agent name resolution
        // broken whenever a reader only queries the group relay. The indexer
        // still rejects NIP-29 kinds, so this union only ever widens where
        // profiles land, never where other kinds are published.
        if matches!(ev, DomainEvent::Profile(_)) {
            let builder = self.wire.encode(ev)?;
            return self.fava.enqueue_profile(builder, keys.public_key());
        }
        let signer = keys.public_key().to_hex();
        match ev {
            DomainEvent::Status(status) => {
                let require_member = status_requires_membership.unwrap_or(true);
                for channel in &status.channels {
                    if !confirmed_scope
                        .is_some_and(|scope| scope.permits(channel, &signer, require_member))
                    {
                        self.verify_publish_scope(channel, &signer, require_member)
                            .await?;
                    }
                }
            }
            DomainEvent::ChatMessage(message) => {
                self.verify_publish_scope(&message.channel, &signer, true)
                    .await?;
            }
            DomainEvent::Reaction(reaction) => {
                self.verify_publish_scope(&reaction.channel, &signer, true)
                    .await?;
            }
            DomainEvent::Profile(_) => unreachable!("profiles return above"),
        }
        let builder = self.wire.encode(ev)?;
        match ev {
            // The several-group write: one kind:30315 replaceable coordinate,
            // one `h` row per channel the session occupies. `nip29::Groups`
            // mints every row; publishing once per channel would
            // have each copy replace the last.
            DomainEvent::Status(status) => self.fava.publish_groups(
                status.channels.iter().cloned(),
                builder,
                keys.public_key(),
            ),
            DomainEvent::ChatMessage(message) => {
                self.fava
                    .publish_group(&message.channel, builder, keys.public_key())
            }
            DomainEvent::Reaction(reaction) => {
                self.fava
                    .publish_group(&reaction.channel, builder, keys.public_key())
            }
            DomainEvent::Profile(_) => unreachable!("profiles return above"),
        }
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}
