//! NMP-backed Nostr acquisition and durable publication.
//!
//! NMP owns relay planning, subscription lifecycle, canonical wire-event
//! deduplication, and acquisition evidence. Mosaico keeps its product read model:
//! delivered events are projected into `state.db` by the existing fabric
//! materializer. NMP also owns every durable write intent, route, receipt, and
//! bounded retry. Shared reads are public because Mosaico-created groups are
//! deliberately public; writes authenticate as the exact event author. The
//! provider supplies product policy and exact host authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use nmp::{AccessContext, Engine, EngineConfig, ObservationCancel, RelayUrl};
use nostr::Keys;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
mod auth;
mod query;
pub(crate) mod read;
mod scrub;
#[cfg(test)]
mod test_io;
mod write;

use auth::IdentityRegistration;
use write::BackgroundReceiptObserver;

const MATERIALIZATION_QUEUE_CAPACITY: usize = 2048;
const MAX_LOCAL_IDENTITIES: usize = 4096;

/// One NMP frame's row transition, carried and applied as a UNIT.
///
/// The unit is the point. A relay republishing an addressable event — every
/// NIP-29 roster change does this — arrives as `Removed(old_id)` and
/// `Added(new_id)` **in the same frame**, and `Removed` carries only an id.
/// The delivered delta order is event-id ascending, not causal
/// (`nmp::runtime::row_channel` re-folds a frame through a `BTreeMap`), so a
/// consumer that acts on each delta as it arrives can render a momentarily
/// empty roster purely on hex ordering. Handing the whole frame across, and
/// applying removals before additions, is the only shape under which the
/// batch's final state is the batch's intent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MaterializationBatch {
    /// Rows that left the observation's row set.
    pub(crate) removed: Vec<nostr::EventId>,
    /// Rows that entered it, carrying the full event.
    pub(crate) added: Vec<nostr::Event>,
}

impl MaterializationBatch {
    fn from_deltas(deltas: &[nmp::RowDelta]) -> Self {
        let mut batch = Self::default();
        for delta in deltas {
            match delta {
                nmp::RowDelta::Added(row) => batch.added.push(row.event.clone()),
                nmp::RowDelta::Removed(id) => batch.removed.push(*id),
                // A relay that already held this exact event id now also
                // serves it. Mosaico's read model is keyed by event id and
                // coordinate and carries no provenance column, and no product
                // surface asks "which relays hold this" — so there is nothing
                // to project. The arm is spelled out rather than folded into a
                // wildcard so the decision stays a decision: NMP owns
                // provenance, and a surface that ever needs it must read
                // `Row.sources` at the point of the question rather than
                // mirror it into a second store here.
                nmp::RowDelta::SourcesGrew { .. } => {}
            }
        }
        batch
    }

    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.added.is_empty()
    }
}

pub(crate) struct NmpHost {
    engine: Engine,
    relays: BTreeSet<RelayUrl>,
    profile_relays: BTreeSet<RelayUrl>,
    identities: Mutex<BTreeMap<nostr::PublicKey, IdentityRegistration>>,
    signing: Mutex<()>,
    subscriptions: Mutex<BTreeMap<String, ObservationCancel>>,
    materialization_tx: Mutex<Option<mpsc::Sender<MaterializationBatch>>>,
    materialization_rx: Mutex<Option<mpsc::Receiver<MaterializationBatch>>>,
    background_receipts: BackgroundReceiptObserver,
    #[cfg(test)]
    test_io: test_io::TestIo,
}

impl NmpHost {
    pub(crate) fn open(
        relays: &[String],
        indexer_relay: Option<&str>,
        store_path: Option<&Path>,
        backend_keys: &Keys,
    ) -> Result<Self> {
        let parsed = relays
            .iter()
            .map(|relay| RelayUrl::parse(relay).with_context(|| format!("invalid relay {relay}")))
            .collect::<Result<BTreeSet<_>>>()?;
        let mut config = EngineConfig {
            store_path: store_path.map(|path| path.to_string_lossy().into_owned()),
            app_relays: relays.to_vec(),
            allowed_local_relay_hosts: local_relay_hosts(parsed.iter()),
            ..EngineConfig::default()
        };
        // A daemon can host many durable agent identities over its lifetime.
        // Keep the registry finite, but do not inherit NMP's small demo default.
        // Each identity consumes one signer registration and one AUTH-policy
        // registration from NMP's shared capability ceiling.
        config.max_auth_capabilities = MAX_LOCAL_IDENTITIES * 2;
        let mut profile_relays = parsed.clone();
        if let Some(indexer) = indexer_relay.filter(|relay| !relay.is_empty()) {
            let parsed_indexer = RelayUrl::parse(indexer)
                .with_context(|| format!("invalid indexer relay {indexer}"))?;
            config
                .allowed_local_relay_hosts
                .extend(local_relay_hosts([&parsed_indexer]));
            config.allowed_local_relay_hosts.sort();
            config.allowed_local_relay_hosts.dedup();
            config.indexer_relays.push(indexer.to_string());
            profile_relays.insert(parsed_indexer);
        }
        let engine = Engine::new(config).context("starting NMP engine")?;
        let (materialization_tx, materialization_rx) =
            mpsc::channel(MATERIALIZATION_QUEUE_CAPACITY);
        let background_receipts = BackgroundReceiptObserver::start()?;
        let host = Self {
            engine,
            relays: parsed,
            profile_relays,
            identities: Mutex::new(BTreeMap::new()),
            signing: Mutex::new(()),
            subscriptions: Mutex::new(BTreeMap::new()),
            materialization_tx: Mutex::new(Some(materialization_tx)),
            materialization_rx: Mutex::new(Some(materialization_rx)),
            background_receipts,
            #[cfg(test)]
            test_io: test_io::TestIo::default(),
        };
        host.ensure_identity(backend_keys)
            .context("registering backend NIP-42 identity")?;
        Ok(host)
    }

    /// Take the one lossless stream feeding Mosaico's canonical read-model
    /// materializer. A bounded channel deliberately backpressures observation
    /// drains instead of dropping canonical additions under a relay burst.
    pub(crate) fn take_materialization_events(
        &self,
    ) -> Result<mpsc::Receiver<MaterializationBatch>> {
        self.materialization_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .context("NMP materialization stream is already owned")
    }

    /// The loopback / private-network hosts the operator opted in by naming
    /// them as relays, in `nmp_network_policy`'s normalized form.
    ///
    /// One allowlist, one vocabulary. A Blossom dial to the same host has to
    /// clear the same destination policy the engine applies to the relay, or
    /// a local test fixture works for one and is refused by the other.
    pub(crate) fn allowed_local_hosts(&self) -> std::collections::BTreeSet<String> {
        local_relay_hosts(self.profile_relays.iter())
            .into_iter()
            .collect()
    }

    /// Open a caller-owned NMP observation. Dropping the returned value closes
    /// it, making this suitable for precise, short-lived correlation queries.
    pub(crate) fn observe(&self, query: &SubscriptionQuery) -> Result<nmp::Subscription> {
        self.observe_with_access(query, AccessContext::Public)
    }

    fn observe_with_access(
        &self,
        query: &SubscriptionQuery,
        access: AccessContext,
    ) -> Result<nmp::Subscription> {
        self.engine
            .observe(self.live_query(query, access)?, None)
            .context("opening NMP observation")
    }

    pub(crate) fn shutdown(&self) {
        let subscriptions = std::mem::take(
            &mut *self
                .subscriptions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for (_, cancel) in subscriptions {
            cancel.cancel();
        }
        self.materialization_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.background_receipts.begin_shutdown();
        self.engine.shutdown();
        self.background_receipts.shutdown();
    }

    pub(crate) fn apply(&self, effect: &SubEffect) -> Result<()> {
        match effect {
            SubEffect::Open { id, query } | SubEffect::Replace { id, query } => {
                self.open_subscription(id, query)
            }
            SubEffect::Close { id } => {
                self.close_subscription(id);
                Ok(())
            }
        }
    }

    fn open_subscription(&self, id: &str, query: &SubscriptionQuery) -> Result<()> {
        let subscription = self
            .observe(query)
            .with_context(|| format!("opening NMP observation {id}"))?;
        let cancel = subscription.cancel_handle();
        let materialization = self
            .materialization_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .context("NMP host is shut down")?;
        std::thread::Builder::new()
            .name(format!("nmp-{id}"))
            .spawn(move || {
                while let Ok(frame) = subscription.recv() {
                    let batch = MaterializationBatch::from_deltas(&frame.deltas);
                    if batch.is_empty() {
                        continue;
                    }
                    if materialization.blocking_send(batch).is_err() {
                        return;
                    }
                }
            })
            .with_context(|| format!("starting NMP observation drain {id}"))?;
        let previous = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .insert(id.to_string(), cancel);
        if let Some(previous) = previous {
            previous.cancel();
        }
        Ok(())
    }

    fn close_subscription(&self, id: &str) {
        if let Some(cancel) = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id)
        {
            cancel.cancel();
        }
    }
}

impl Drop for NmpHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn local_relay_hosts<'a>(relays: impl IntoIterator<Item = &'a RelayUrl>) -> Vec<String> {
    relays
        .into_iter()
        .filter_map(nmp_grammar::relay::relay_host_key)
        .filter(|host| {
            nmp_network_policy::classify_bare_host(host) == nmp_network_policy::HostClass::Local
        })
        // Onion routing is local in transport terms but not a local-network
        // SSRF opt-in. NMP handles it as a separate trust class.
        .filter(|host| !host.ends_with(".onion"))
        .collect()
}

#[cfg(test)]
mod tests;
