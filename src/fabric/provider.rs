//! Phase 5: `Nip29Provider` — concrete provider wrapping delivery, wire
//! codec, materializer, and lifecycle in one place.

mod group_management;
mod group_state;
mod materialization;
mod readiness;

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate, ChannelReadiness};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::nostr_delivery::NostrDelivery;
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::state::Store;
use crate::transport::Transport;
use anyhow::Result;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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
    pub delivery: NostrDelivery,
    pub wire: Nip29WireCodec,
    /// Shared store Arc — same handle as `DaemonState.store`. No new Connection.
    pub store: Arc<Mutex<Store>>,
    /// Same Arc as `DaemonState.transport` — used for lifecycle publishes only.
    pub transport: Arc<Transport>,
    /// Backend management signing key (`tenexPrivateKey`).
    pub management_nsec: Option<String>,
    /// Human operator key (`userNsec`) for self-granting the management key.
    pub user_nsec: Option<String>,
    /// Whitelisted human pubkeys (hex) that should hold admin in owned groups.
    pub whitelisted_pubkeys: Vec<String>,
    /// Stable hash of the sorted relay URL set.
    pub provider_instance: String,
    /// TTL'd in-process cache of which channels are known-ready.
    pub readiness: Arc<ChannelReadiness>,
}

impl Nip29Provider {
    pub fn new(
        transport: Arc<Transport>,
        store: Arc<Mutex<Store>>,
        management_nsec: Option<String>,
        user_nsec: Option<String>,
        whitelisted_pubkeys: Vec<String>,
        relays: &[String],
    ) -> Self {
        let delivery = NostrDelivery::new(transport.clone());
        let wire = Nip29WireCodec;
        let provider_instance = derive_provider_instance(relays);
        Self {
            delivery,
            wire,
            store,
            transport,
            management_nsec,
            user_nsec,
            whitelisted_pubkeys,
            provider_instance,
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

    /// Encode + sign + publish ONE domain event.
    pub async fn publish(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        // kind:0 profiles route to the indexer relay only (purplepag.es). The
        // main NIP-29 relay rejects kind:0 (it's not a NIP-29 kind), and the
        // indexer rejects NIP-29 kinds — so the two must never share a publish.
        // `publish_event_to` targets the indexer URL explicitly when configured,
        // and falls back to the WRITE pool when no indexer is set.
        if matches!(ev, DomainEvent::Profile(_)) {
            let builder = self.wire.encode(ev)?;
            let signed = self.transport.sign(builder, keys).await?;
            let urls: Vec<String> = self
                .transport
                .indexer_url()
                .map(|u| vec![u.to_string()])
                .unwrap_or_default();
            return self.transport.publish_event_to(&signed, &urls).await;
        }
        if let Some(ch) = ev.channel() {
            let agent_pubkey = keys.public_key().to_hex();
            let parent = self
                .with_store(|s| s.channel_parent(ch).unwrap_or(None))
                .filter(|p| !p.is_empty());
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
        self.transport.publish_signed(builder, keys).await
    }

    /// Like [`publish`], but fails when no relay accepted the event.
    pub async fn publish_checked(
        &self,
        ev: &DomainEvent,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        if let Some(ch) = ev.channel() {
            let agent_pubkey = keys.public_key().to_hex();
            let parent = self
                .with_store(|s| s.channel_parent(ch).unwrap_or(None))
                .filter(|p| !p.is_empty());
            let ctx = ChannelCtx {
                channel: ch,
                expect_member: &agent_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            };
            if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
                anyhow::bail!(
                    "publish_checked: channel {ch} is not verified (ChannelGate::Degraded) — refusing to publish into an unverified channel"
                );
            }
        }
        let builder = self.wire.encode(ev)?;
        self.transport.publish_signed_checked(builder, keys).await
    }

    /// Read an event back by id to confirm it is retrievable from the relay.
    pub async fn is_retrievable(&self, id: nostr_sdk::prelude::EventId, timeout: Duration) -> bool {
        use nostr_sdk::prelude::Filter;
        let f = Filter::new().id(id).limit(1);
        match self.transport.fetch(f, timeout).await {
            Ok(evs) => !evs.is_empty(),
            Err(e) => {
                tracing::error!(
                    event_id = %id,
                    error = %format!("{e:#}"),
                    "is_retrievable: relay read-back failed — treating as not-retrievable"
                );
                false
            }
        }
    }

    /// Connectivity probe: publish a uniquely-tagged throwaway note and read it back.
    pub async fn doctor_probe(&self) -> (String, String) {
        use nostr_sdk::prelude::{Alphabet, EventBuilder, Filter, Kind, SingleLetterTag, Tag};
        let t = format!("te-doctor-{}", crate::util::now_secs());
        let publish = async {
            let builder = EventBuilder::new(Kind::from(1u16), format!("tenex-edge doctor {t}"))
                .tags([Tag::parse(["h", &t])?]);
            self.transport.publish_builder_checked(builder).await
        }
        .await;
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

    /// Subscribe to subgroup orchestration events p-tagged to this backend identity.
    pub async fn subscribe_backend_orchestration(&self, backend_pubkey: &str) -> Result<()> {
        use nostr_sdk::prelude::{Filter, PublicKey};
        if let Ok(pk) = PublicKey::from_hex(backend_pubkey) {
            let f = Filter::new()
                .kind(crate::fabric::nip29::wire::kind(
                    crate::fabric::nip29::wire::KIND_CHAT,
                ))
                .pubkey(pk);
            self.transport.subscribe(vec![f]).await?;
        }
        Ok(())
    }

    /// Forward a `Scope` subscription to the underlying delivery layer.
    pub async fn subscribe(&self, scope: crate::fabric::Scope) -> Result<()> {
        self.delivery.subscribe(scope).await
    }

    /// Publish ONE kind:30315 status for a session. The event is replaceable by
    /// `(author pubkey, session_id)` and targets every channel in `status.channels`.
    pub async fn set_status(
        &self,
        status: &crate::domain::Status,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let agent_pubkey = keys.public_key().to_hex();
        for channel in &status.channels {
            let parent = self
                .with_store(|s| s.channel_parent(channel).unwrap_or(None))
                .filter(|p| !p.is_empty());
            let ctx = ChannelCtx {
                channel,
                expect_member: &agent_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            };
            if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
                anyhow::bail!(
                    "set_status: channel {channel} is not verified (ChannelGate::Degraded) — refusing to publish status into an unverified channel",
                );
            }
        }
        let builder = self.wire.encode(&DomainEvent::Status(status.clone()))?;
        self.transport.publish_signed_checked(builder, keys).await
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}

/// Derive a stable `provider_instance` string from the relay URL set.
fn derive_provider_instance(relays: &[String]) -> String {
    let mut sorted: Vec<&str> = relays.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    let joined = sorted.join("|");
    let mut h = DefaultHasher::new();
    joined.hash(&mut h);
    format!("{:016x}", h.finish())
}
