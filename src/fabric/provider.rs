//! Phase 5: `Nip29Provider` — concrete provider wrapping delivery, wire
//! codec, materializer, and lifecycle in one place.

mod group_management;
mod readiness;

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate, ChannelReadiness};
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::nostr_delivery::NostrDelivery;
use crate::fabric::{MaterializationOutcome, RawEnvelope, WireCodec};
use crate::state::Store;
use crate::transport::Transport;
use anyhow::{Context, Result};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
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
        if let Some(ch) = ev.channel() {
            let agent_pubkey = keys.public_key().to_hex();
            let parent = self.with_store(|s| s.channel_parent(ch).unwrap_or(None))
                .filter(|p| !p.is_empty());
            let ctx = ChannelCtx {
                channel: ch,
                expect_member: &agent_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
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
            let parent = self.with_store(|s| s.channel_parent(ch).unwrap_or(None))
                .filter(|p| !p.is_empty());
            let ctx = ChannelCtx {
                channel: ch,
                expect_member: &agent_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
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

    /// Fetch the relay's live state for `group`: `(exists, roles, members)`.
    ///
    /// Legacy 3-tuple surface kept for callers that cannot distinguish a relay
    /// fetch failure from genuine absence. A transport error is logged loudly and
    /// surfaced here as `(false, empty, empty)` — but provisioning decisions MUST
    /// use [`try_fetch_group_state`] instead, so a fetch failure never masquerades
    /// as "group absent" and triggers spurious re-creation (relay-projection rule).
    pub async fn fetch_group_state(
        &self,
        group: &str,
    ) -> (bool, HashMap<String, String>, HashSet<String>) {
        match self.try_fetch_group_state(group).await {
            Ok(state) => state,
            Err(e) => {
                tracing::error!(
                    group,
                    error = %format!("{e:#}"),
                    "fetch_group_state: relay fetch failed — returning empty state; DO NOT treat as group-absent"
                );
                (false, HashMap::new(), HashSet::new())
            }
        }
    }

    /// Like [`fetch_group_state`] but surfaces a relay/transport fetch failure as
    /// `Err`, so the provisioning path can degrade WITHOUT attempting group
    /// creation. `Ok((false, ..))` means the group is genuinely absent on the relay.
    pub(in crate::fabric::provider) async fn try_fetch_group_state(
        &self,
        group: &str,
    ) -> Result<(bool, HashMap<String, String>, HashSet<String>)> {
        use crate::fabric::nip29::wire::{
            KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA,
        };
        use nostr_sdk::prelude::Filter;
        let filter = Filter::new()
            .kinds([
                crate::fabric::nip29::wire::kind(KIND_GROUP_METADATA),
                crate::fabric::nip29::wire::kind(KIND_GROUP_ADMINS),
                crate::fabric::nip29::wire::kind(KIND_GROUP_MEMBERS),
            ])
            .identifier(group);
        let state_evs = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .context("fetch_group_state: relay fetch of group state failed")?;

        let newest = |k: u16| {
            state_evs
                .iter()
                .filter(|e| e.kind.as_u16() == k)
                .max_by_key(|e| e.created_at.as_secs())
        };
        let group_exists = newest(KIND_GROUP_METADATA).is_some()
            || newest(KIND_GROUP_ADMINS).is_some()
            || newest(KIND_GROUP_MEMBERS).is_some();

        let mut roles: HashMap<String, String> = HashMap::new();
        if let Some(ev) = newest(KIND_GROUP_ADMINS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        roles.insert(
                            pk.clone(),
                            s.get(2).cloned().unwrap_or_else(|| "member".to_string()),
                        );
                    }
                }
            }
        }

        let mut members: HashSet<String> = HashSet::new();
        if let Some(ev) = newest(KIND_GROUP_MEMBERS) {
            for t in ev.tags.iter() {
                let s = t.as_slice();
                if s.first().map(String::as_str) == Some("p") {
                    if let Some(pk) = s.get(1) {
                        members.insert(pk.clone());
                    }
                }
            }
        }
        Ok((group_exists, roles, members))
    }

    /// Convenience: just the role map (kind:39001) for `group`.
    pub async fn fetch_group_roles(&self, group: &str) -> HashMap<String, String> {
        self.fetch_group_state(group).await.1
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

    /// The `parent` group id declared in `group`'s relay-authored kind:39000 metadata.
    pub async fn fetch_group_parent(&self, group: &str) -> Option<String> {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        use nostr_sdk::prelude::Filter;
        let filter = Filter::new()
            .kind(crate::fabric::nip29::wire::kind(KIND_GROUP_METADATA))
            .identifier(group);
        let evs = match self.transport.fetch(filter, Duration::from_secs(5)).await {
            Ok(evs) => evs,
            Err(e) => {
                // A fetch failure is not "no parent declared"; surface it loudly
                // rather than silently returning None.
                tracing::error!(
                    group,
                    error = %format!("{e:#}"),
                    "fetch_group_parent: relay fetch failed — could not determine parent (returning None)"
                );
                return None;
            }
        };
        let newest = evs.iter().max_by_key(|e| e.created_at.as_secs())?;
        newest.tags.iter().find_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("parent") {
                s.get(1).cloned()
            } else {
                None
            }
        })
    }

    /// Forward a `Scope` subscription to the underlying delivery layer.
    pub async fn subscribe(&self, scope: crate::fabric::Scope) -> Result<()> {
        self.delivery.subscribe(scope).await
    }

    /// Publish ONE kind:30315 status for a session.
    pub async fn set_status(
        &self,
        status: &crate::domain::Status,
        keys: &nostr_sdk::prelude::Keys,
    ) -> Result<nostr_sdk::prelude::EventId> {
        let agent_pubkey = keys.public_key().to_hex();
        let parent = self.with_store(|s| s.channel_parent(&status.project).unwrap_or(None))
            .filter(|p| !p.is_empty());
        let ctx = ChannelCtx {
            channel: &status.project,
            expect_member: &agent_pubkey,
            parent_hint: parent.as_deref(),
            name: None,
        };
        if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
            anyhow::bail!(
                "set_status: channel {} is not verified (ChannelGate::Degraded) — refusing to publish status into an unverified channel",
                status.project
            );
        }
        let builder = self.wire.encode(&DomainEvent::Status(status.clone()))?;
        self.transport.publish_signed(builder, keys).await
    }

    /// Fetch the relay-authored kind:39000 for ONE `group` and materialize it into
    /// `relay_channels` via the single inbound materializer. Returns `true` once a
    /// row for `group` exists in the cache. This is how a just-created group enters
    /// the cache: by reading back the relay's own metadata — never by a local
    /// optimistic write.
    pub async fn fetch_and_materialize_channel(&self, group: &str) -> bool {
        use crate::fabric::nip29::materializer::Nip29Materializer;
        use crate::fabric::nip29::wire::{kind, KIND_GROUP_METADATA};
        use nostr_sdk::prelude::Filter;
        let filter = Filter::new()
            .kind(kind(KIND_GROUP_METADATA))
            .identifier(group);
        let evs = match self.transport.fetch(filter, Duration::from_secs(5)).await {
            Ok(evs) => evs,
            Err(e) => {
                // Relay fetch failed: surface it loudly. We fall through to the
                // existing-cache check rather than fabricating a row.
                tracing::error!(
                    group,
                    error = %format!("{e:#}"),
                    "fetch_and_materialize_channel: relay fetch of kind:39000 failed — cannot materialize"
                );
                Vec::new()
            }
        };
        if let Some(newest) = evs.iter().max_by_key(|e| e.created_at.as_secs()) {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, newest));
        }
        self.with_store(|s| s.get_channel(group).ok().flatten().is_some())
    }

    /// Fetch all kind:39000 events from the relay and materialize them into the
    /// `relay_channels` cache via the single inbound materializer.
    pub async fn refresh_project_list(&self) -> Result<()> {
        use crate::fabric::nip29::materializer::Nip29Materializer;
        use nostr_sdk::prelude::{Filter, Kind};
        let filter = Filter::new().kind(Kind::from(39000u16)).limit(200);
        let events = self
            .transport
            .fetch(filter, Duration::from_secs(5))
            .await
            .context("refresh_project_list: relay fetch of kind:39000 list failed")?;
        for ev in &events {
            self.with_store(|s| Nip29Materializer::materialize_channel(s, ev));
        }
        Ok(())
    }

    /// Decode one raw envelope and apply all store side-effects.
    pub fn materialize(
        &self,
        env: &RawEnvelope,
        hosted: &[String],
        now: u64,
        store: &Store,
    ) -> MaterializationOutcome {
        crate::fabric::materialize(env, hosted, now, &self.provider_instance, store)
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
