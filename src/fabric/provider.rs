//! `Nip29Provider` — concrete NIP-29 wire, materializer, and lifecycle boundary.

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

    /// Connectivity probe: publish a uniquely-tagged note to an existing group
    /// this management identity belongs to, then read that exact marker back.
    pub async fn doctor_probe(&self) -> (String, String) {
        let marker = format!("mosaico-doctor-{}", crate::util::opaque_group_id());
        let group = match self.doctor_probe_group().await {
            Ok(Some(group)) => group,
            Ok(None) => return self.doctor_read_only().await,
            Err(error) => {
                let error = format!("ERR {error:#}");
                return (error.clone(), error);
            }
        };
        let keys = match self.management_keys() {
            Some(keys) => keys,
            None => {
                let error = "ERR management signing identity is unavailable".to_string();
                return (error.clone(), error);
            }
        };
        let publish = self
            .nmp
            .publish_group_builder(doctor_probe_builder(&group, &marker), &keys, true)
            .await;
        let publish = match publish {
            Ok(id) => format!("OK ({})", crate::util::pubkey_short(&id.to_hex())),
            Err(e) => format!("ERR {e:#}"),
        };
        let f = doctor_probe_filter(&group, &marker);
        let readback = match self.nmp.fetch_group(f, 5, Duration::from_secs(5)).await {
            Ok(evs) => format!("{} event(s) with #h={group} #t={marker}", evs.len()),
            Err(e) => format!("ERR {e:#}"),
        };
        (publish, readback)
    }

    async fn doctor_probe_group(&self) -> Result<Option<String>> {
        let pubkey = self
            .management_pubkey()
            .ok_or_else(|| anyhow::anyhow!("management signing identity is unavailable"))?;
        let candidates = self.with_store(|store| store.list_channels_where_member(&pubkey))?;
        if candidates.is_empty() {
            return Ok(None);
        }

        let mut fetch_errors = Vec::new();
        for group in candidates {
            match self.fetch_group_state(&group).await {
                Ok((true, roles, members))
                    if roles.contains_key(&pubkey) || members.contains(&pubkey) =>
                {
                    return Ok(Some(group));
                }
                Ok(_) => {}
                Err(error) => fetch_errors.push(format!("{group}: {error:#}")),
            }
        }
        if !fetch_errors.is_empty() {
            anyhow::bail!(
                "could not verify an existing authorized NIP-29 group: {}",
                fetch_errors.join("; ")
            );
        }
        Ok(None)
    }

    async fn doctor_read_only(&self) -> (String, String) {
        use crate::fabric::nip29::wire::KIND_GROUP_METADATA;
        let reason = "SKIP no existing materialized NIP-29 group authorizes the management identity; publish probe not attempted";
        let filter = crate::nmp_host::read::filter(&[KIND_GROUP_METADATA], &[], &[])
            .expect("static NMP metadata filter");
        let read = self
            .nmp
            .fetch_group(filter, 1, Duration::from_secs(5))
            .await;
        let read = match read {
            Ok(events) => format!(
                "SKIP publish readback; relay read OK ({} metadata event(s))",
                events.len()
            ),
            Err(error) => format!("ERR relay read failed: {error:#}"),
        };
        (reason.to_string(), read)
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
}

fn doctor_probe_builder(group: &str, marker: &str) -> nostr::EventBuilder {
    nostr::EventBuilder::new(nostr::Kind::from(1u16), format!("mosaico doctor {marker}")).tags([
        nostr::Tag::parse(["h", group]).expect("static h tag"),
        nostr::Tag::parse(["t", marker]).expect("static t tag"),
    ])
}

fn doctor_probe_filter(group: &str, marker: &str) -> nmp::Filter {
    crate::nmp_host::read::filter(
        &[1],
        &[],
        &[('h', group.to_string()), ('t', marker.to_string())],
    )
    .expect("static NMP doctor filter")
}

#[cfg(test)]
mod tests {
    #[test]
    fn doctor_readback_is_scoped_to_existing_group_and_unique_marker() {
        let filter = super::doctor_probe_filter("existing-workspace", "mosaico-doctor-test");
        assert_eq!(filter.tags.len(), 2);
    }
}
