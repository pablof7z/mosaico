//! `Nip29Provider` — concrete NIP-29 wire, materializer, and lifecycle boundary.

pub(crate) mod chat;
mod doctor;
mod group_management;
mod group_state;
mod group_topology;
mod materialization;
mod membership_confirmation;
mod profiles;
mod publish_gate;
mod reactions;
mod readiness;

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::ChannelReadiness;
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::nmp_host::NmpHost;
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
    /// NMP owns every relay read and write, signer selection, routing, and receipt.
    pub(crate) nmp: Arc<NmpHost>,
    /// Backend management signing key (`mosaicoPrivateKey`). Missing keys are
    /// generated and persisted by the shared readiness/provisioning path.
    management_nsec: Mutex<Option<String>>,
    /// Human operator key (`userNsec`) for self-granting the management key.
    pub user_nsec: Option<String>,
    /// Whitelisted human pubkeys (hex) that should hold admin in owned groups.
    pub whitelisted_pubkeys: Vec<String>,
    /// TTL'd in-process cache of which channels are known-ready.
    pub readiness: Arc<ChannelReadiness>,
}

impl Nip29Provider {
    pub(crate) fn new(
        nmp: Arc<NmpHost>,
        store: Arc<Mutex<Store>>,
        management_nsec: Option<String>,
        user_nsec: Option<String>,
        whitelisted_pubkeys: Vec<String>,
    ) -> Self {
        let wire = Nip29WireCodec;
        Self {
            wire,
            store,
            nmp,
            management_nsec: Mutex::new(management_nsec),
            user_nsec,
            whitelisted_pubkeys,
            readiness: Arc::new(ChannelReadiness::default()),
        }
    }

    pub fn name(&self) -> &'static str {
        "nip29"
    }

    /// Encode a domain event to an `EventBuilder` via the NIP-29 wire codec.
    pub fn encode(&self, ev: &DomainEvent) -> Result<nostr::EventBuilder> {
        self.wire.encode(ev)
    }

    /// Decode a raw envelope to a domain event via the NIP-29 wire codec.
    pub fn decode(&self, env: &RawEnvelope) -> Option<DomainEvent> {
        self.wire.decode(env)
    }

    /// Encode, sign, and durably enqueue one domain event. Relay delivery is
    /// always owned by NMP after this local acceptance boundary.
    pub async fn enqueue(&self, ev: &DomainEvent, keys: &nostr::Keys) -> Result<nostr::EventId> {
        // kind:0 profiles route to BOTH the indexer relay (purplepag.es) AND
        // the main NIP-29 relay(s) — the group relay accepts kind:0 fine, so
        // relying on the indexer alone leaves backend/agent name resolution
        // broken whenever a reader only queries the group relay. The indexer
        // still rejects NIP-29 kinds, so this union only ever widens where
        // profiles land, never where other kinds are published.
        if matches!(ev, DomainEvent::Profile(_)) {
            let builder = self.wire.encode(ev)?;
            let signed = self.nmp.sign_event(builder, keys).await?;
            let event_id = self.nmp.enqueue_profile_event(&signed)?;
            self.with_store(|store| {
                self.materialize(&RawEnvelope::Nostr(signed), store);
            });
            return Ok(event_id);
        }
        let signer = keys.public_key().to_hex();
        match ev {
            DomainEvent::Status(status) => {
                let expired = matches!(
                    status.expires_at,
                    Some(expires_at) if expires_at <= crate::util::now_secs()
                );
                for channel in &status.channels {
                    self.verify_publish_scope(channel, &signer, !expired)
                        .await?;
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
            // The multi-group write. Its `h` rows are already in the bytes the
            // wire codec composed -- see `NmpHost::enqueue_multi_group_event`
            // for why no NMP door can mint them.
            DomainEvent::Status(_) => {
                let signed = self.nmp.sign_event(builder, keys).await?;
                self.nmp.enqueue_multi_group_event(&signed)
            }
            DomainEvent::ChatMessage(message) => {
                self.nmp
                    .publish_group_builder(&message.channel, builder, keys)
            }
            DomainEvent::Reaction(reaction) => {
                self.nmp
                    .publish_group_builder(&reaction.channel, builder, keys)
            }
            DomainEvent::Profile(_) => unreachable!("profiles return above"),
        }
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}
