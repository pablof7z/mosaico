//! `Nip29Provider` — concrete NIP-29 wire, materializer, and lifecycle boundary.

mod agent_roster;
pub(crate) mod chat;
mod group_management;
mod group_state;
mod group_topology;
mod materialization;
mod membership_confirmation;
mod profiles;
mod reactions;
mod readiness;

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate, ChannelReadiness};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::nmp_host::NmpHost;
use crate::state::Store;
use crate::transport::Transport;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// Narrow direct client for bounded reads and diagnostics.
    pub(crate) transport: Arc<Transport>,
    /// NMP owns all durable writes, signer selection, routing, and receipts.
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
        transport: Arc<Transport>,
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
            transport,
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
    pub fn encode(&self, ev: &DomainEvent) -> Result<nostr_sdk::EventBuilder> {
        self.wire.encode(ev)
    }

    /// Decode a raw envelope to a domain event via the NIP-29 wire codec.
    pub fn decode(&self, env: &RawEnvelope) -> Option<DomainEvent> {
        self.wire.decode(env)
    }

    /// Encode, sign, and durably enqueue one domain event. Relay delivery is
    /// always owned by NMP after this local acceptance boundary.
    pub async fn enqueue(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        // kind:0 profiles route to BOTH the indexer relay (purplepag.es) AND
        // the main NIP-29 relay(s) — the group relay accepts kind:0 fine, so
        // relying on the indexer alone leaves backend/agent name resolution
        // broken whenever a reader only queries the group relay. The indexer
        // still rejects NIP-29 kinds, so this union only ever widens where
        // profiles land, never where other kinds are published.
        if matches!(ev, DomainEvent::Profile(_)) {
            let builder = self.wire.encode(ev)?;
            let signed = self.nmp.sign_event(builder, keys).await?;
            return self.nmp.enqueue_profile_event(&signed);
        }
        if let Some(ch) = ev.channel() {
            let agent_pubkey = keys.public_key().to_hex();
            let parent = readiness::stored_parent_hint(self, ch)?;
            let ctx = ChannelCtx {
                channel: ch,
                expect_member: &agent_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            };
            if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
                anyhow::bail!(
                    "publish: channel {ch} is not verified (ChannelGate::Degraded) — refusing to publish into an unverified channel"
                );
            }
        }
        let builder = self.wire.encode(ev)?;
        if matches!(ev, DomainEvent::Status(_)) {
            let signed = self.nmp.sign_event(builder, keys).await?;
            return self.nmp.enqueue_group_event(&signed);
        }
        self.nmp.publish_group_builder(builder, keys, false).await
    }

    /// Connectivity probe: publish a uniquely-tagged throwaway note and read it back.
    pub async fn doctor_probe(&self) -> (String, String) {
        use nostr_sdk::prelude::{Alphabet, Filter, Kind, SingleLetterTag};
        let t = format!("mosaico-doctor-{}", crate::util::now_secs());
        let publish = self.transport.publish_probe_checked(&t).await;
        let publish = match publish {
            Ok(id) => format!("OK ({})", crate::util::pubkey_short(&id.to_hex())),
            Err(e) => format!("ERR {e:#}"),
        };
        tokio::time::sleep(Duration::from_secs(1)).await;
        let f = Filter::new()
            .kind(Kind::from(1u16))
            .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &t)
            .limit(5);
        let readback = match self.transport.fetch(f, Duration::from_secs(5)).await {
            Ok(evs) => format!("{} event(s) with #h={t}", evs.len()),
            Err(e) => format!("ERR {e:#}"),
        };
        (publish, readback)
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}
