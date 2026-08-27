//! Fava-backed Nostr acquisition and durable publication.
//!
//! Fava owns relay planning, subscription lifecycle, canonical wire-event
//! deduplication, current event state, and NIP-42 authentication. Mosaico
//! keeps only process-local presentation projections and genuinely local
//! durable facts. Fava also owns every durable write intent, route, receipt,
//! and bounded retry. Shared reads are public because Mosaico-created groups
//! are deliberately public; writes authenticate as the exact event author.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use fava::{EventValue, Fava, FavaBuilder, Observation, ObservationClosed, RelayUrl};
use fava_delivery_standard::StandardDeliveryPolicy;
use fava_event_cache_memory::MemoryEventCache;
use fava_publisher_nip01::Nip01Publisher;
use fava_query_standard::StandardQueryEvaluator;
use fava_router_app_relays::AppRelayRouter;
use fava_signer_local::LocalSigner;
use fava_subscriptions_no_grouping::planner;
use fava_transport_websocket::WebSocketTransport;
use fava_write_store_memory::MemoryWriteStore;
use fava_write_store_redb::RedbWriteStore;
use nostr::key::Keys;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
use crate::row::{Row, RowDelta};

mod auth;
mod query;
pub(crate) mod read;
mod scrub;
pub(crate) mod store;
#[cfg(feature = "stress-harness")]
mod stress;
#[cfg(test)]
mod test_io;
pub(crate) mod write;

use crate::nmp_views::{NmpViews, RowTransition};

mod profile_feed;
pub(crate) use profile_feed::ProfileFeed;
#[cfg(test)]
mod test_relay;
#[cfg(test)]
pub(crate) use test_relay::PlainRelay;

const VIEW_TRANSITION_QUEUE_CAPACITY: usize = 2048;

/// Maximum row target for a windowed broad `#h` observation (mosaico#837).
/// Fava delivers complete snapshots rather than windowed streams, so this
/// constant is exposed for callers that expect it but `request_rows` is a
/// no-op: all retained rows are already in the `RecentRows` store.
pub(crate) const OBSERVATION_WINDOW_MAX: usize = 10_000;

struct ActiveObservation {
    // Task owns the Observation; aborting drops it → Observation::drop → close().
    _task: tokio::task::JoinHandle<()>,
}

pub(crate) struct NmpHost {
    pub(crate) fava: Fava,
    relays: BTreeSet<RelayUrl>,
    pub(crate) profile_relays: BTreeSet<RelayUrl>,
    subscriptions: Mutex<BTreeMap<String, ActiveObservation>>,
    next_observation_generation: Mutex<u64>,
    views: Arc<NmpViews>,
    transition_tx: Mutex<Option<mpsc::Sender<RowTransition>>>,
    transition_rx: Mutex<Option<mpsc::Receiver<RowTransition>>>,
    #[cfg(test)]
    pub(crate) test_io: test_io::TestIo,
}

impl NmpHost {
    /// Open the durable store and the fava engine over it.
    ///
    /// A store refusal comes back as an error carrying the typed
    /// [`store::StoreCondition`] as its source, so a caller decides what to do
    /// from [`store::StoreCondition::of_open_error`] — never by reading the
    /// message. The distinction that matters is the one between a superseded
    /// schema epoch, which only discarding the store fixes, and every other
    /// refusal, which discarding the store makes permanently worse.
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
        let mut profile_relays = parsed.clone();
        if let Some(indexer) = indexer_relay.filter(|relay| !relay.is_empty()) {
            let parsed_indexer = RelayUrl::parse(indexer)
                .with_context(|| format!("invalid indexer relay {indexer}"))?;
            profile_relays.insert(parsed_indexer);
        }
        let fava = build_fava(&parsed, store_path, backend_keys)?;
        let (transition_tx, transition_rx) = mpsc::channel(VIEW_TRANSITION_QUEUE_CAPACITY);
        Ok(Self {
            fava,
            relays: parsed,
            profile_relays,
            subscriptions: Mutex::new(BTreeMap::new()),
            next_observation_generation: Mutex::new(0),
            views: Arc::new(NmpViews::default()),
            transition_tx: Mutex::new(Some(transition_tx)),
            transition_rx: Mutex::new(Some(transition_rx)),
            #[cfg(test)]
            test_io: test_io::TestIo::default(),
        })
    }

    /// The product-side view projections accumulated from fava deliveries.
    pub(crate) fn views(&self) -> &NmpViews {
        &self.views
    }

    pub(crate) fn views_handle(&self) -> Arc<NmpViews> {
        self.views.clone()
    }

    /// Take the one lossless stream feeding Mosaico's product side effects.
    /// A bounded channel deliberately backpressures observation drains instead
    /// of dropping transitions under a relay burst.
    pub(crate) fn take_view_transitions(&self) -> Result<mpsc::Receiver<RowTransition>> {
        self.transition_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .context("NMP view transition stream is already owned")
    }

    /// The loopback / private-network hosts the operator opted in by naming
    /// them as relays.
    ///
    /// One allowlist, one vocabulary. A Blossom dial to the same host has to
    /// clear the same destination policy the engine applies to the relay, or
    /// a local test fixture works for one and is refused by the other.
    pub(crate) fn allowed_local_hosts(&self) -> BTreeSet<String> {
        local_relay_hosts(self.profile_relays.iter())
            .into_iter()
            .collect()
    }

    /// Open a caller-owned fava observation. Dropping the returned value closes
    /// it, making this suitable for precise, short-lived correlation queries.
    pub(crate) fn observe(&self, query: &SubscriptionQuery) -> Result<Observation> {
        let q = self.fava_query(query)?;
        observe_sync(&self.fava, q)
    }

    /// Open a retained observation over all group records (39000/39001/39002).
    /// Sync entry point; bridges via the current tokio runtime handle.
    pub(crate) fn open_group_records_observation(&self) -> Result<Observation> {
        let query = self.all_group_records_query()?;
        observe_sync(&self.fava, query)
    }

    /// Open a retained observation over one group's records (async entry point).
    pub(crate) async fn open_one_group_records_observation(
        &self,
        group: &str,
    ) -> Result<Observation> {
        let query = self.one_group_records_query(group)?;
        self.fava
            .observe(query)
            .await
            .context("opening one-group-records observation")
    }

    pub(crate) fn shutdown(&self) {
        let subscriptions = std::mem::take(
            &mut *self
                .subscriptions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for (_, active) in subscriptions {
            active._task.abort();
        }
        self.transition_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
    }

    /// No-op under fava: fava delivers complete snapshots rather than windowed
    /// streams, so all retained rows are already in `RecentRows`. The constant
    /// `OBSERVATION_WINDOW_MAX` is exposed for callers that need the max value.
    pub(crate) fn request_rows(&self, _id: &str, _at_least: usize) -> Result<()> {
        Ok(())
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
        let q = self
            .fava_query(query)
            .with_context(|| format!("building fava query for subscription {id}"))?;
        let observation = observe_sync(&self.fava, q)
            .with_context(|| format!("opening fava observation {id}"))?;
        let generation = self.allocate_observation_generation();
        self.views.begin_observation(id, generation);
        let transitions = self
            .transition_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .context("NMP host is shut down")?;
        let views = self.views.clone();
        let observation_id = id.to_string();
        // Drop previous before spawning; dropping ActiveObservation aborts its
        // task, which drops the Observation → Observation::Drop → close().
        self.subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id);
        let task = tokio::spawn(drain_subscription(
            observation,
            views,
            transitions,
            observation_id,
            generation,
        ));
        self.subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .insert(id.to_string(), ActiveObservation { _task: task });
        Ok(())
    }

    fn close_subscription(&self, id: &str) {
        // Dropping the ActiveObservation aborts its task → drops Observation → close().
        self.subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id);
    }

    fn allocate_observation_generation(&self) -> u64 {
        let mut next = self
            .next_observation_generation
            .lock()
            .expect("NMP observation generation mutex poisoned");
        *next = next
            .checked_add(1)
            .expect("NMP observation generation exhausted");
        *next
    }
}

impl Drop for NmpHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn drain_subscription(
    mut observation: Observation,
    views: Arc<NmpViews>,
    transitions: mpsc::Sender<RowTransition>,
    observation_id: String,
    generation: u64,
) {
    let mut prev_ids: BTreeSet<nostr::event::EventId> = BTreeSet::new();
    loop {
        match observation.changed().await {
            Ok(snapshot) => {
                let curr_events: BTreeMap<nostr::event::EventId, Row> = snapshot
                    .events
                    .iter()
                    .filter_map(|record| match record.event() {
                        EventValue::Signed(e) => Some((e.id, Row { event: e.clone() })),
                        EventValue::Unsigned(_) => None,
                    })
                    .collect();
                let curr_ids: BTreeSet<nostr::event::EventId> = curr_events.keys().copied().collect();
                let mut deltas = Vec::new();
                for removed_id in prev_ids.difference(&curr_ids) {
                    deltas.push(RowDelta::Removed(*removed_id));
                }
                for added_id in curr_ids.difference(&prev_ids) {
                    if let Some(row) = curr_events.get(added_id).cloned() {
                        deltas.push(RowDelta::Added(row));
                    }
                }
                prev_ids = curr_ids;
                if !deltas.is_empty() {
                    let transition = views.apply_frame(&observation_id, generation, deltas);
                    if !transition.is_empty() && transitions.send(transition).await.is_err() {
                        return;
                    }
                }
            }
            Err(ObservationClosed) => break,
        }
    }
    let transition = views.close_observation(&observation_id, generation);
    if !transition.is_empty() {
        let _ = transitions.send(transition).await;
    }
}

/// Bridge a sync caller into an async `fava.observe()` via the current runtime.
///
/// Uses a new OS thread so that `Handle::block_on` is not called from within an
/// existing async task (which would panic on both single- and multi-thread
/// runtimes).
pub(crate) fn observe_sync(fava: &Fava, query: fava::Query) -> Result<Observation> {
    let handle = tokio::runtime::Handle::try_current()
        .context("fava observe: no tokio runtime available")?;
    let fava = fava.clone();
    std::thread::spawn(move || handle.block_on(fava.observe(query)))
        .join()
        .map_err(|_| anyhow::anyhow!("fava observe thread panicked"))?
        .context("opening fava observation")
}

fn build_fava(relays: &BTreeSet<RelayUrl>, store_path: Option<&Path>, keys: &Keys) -> Result<Fava> {
    let event_cache = Arc::new(MemoryEventCache::default());
    let evaluator = Arc::new(StandardQueryEvaluator);
    let sub_planner = Arc::new(planner());
    let transport = Arc::new(WebSocketTransport::new());
    let router = Arc::new(AppRelayRouter::new("app", relays.iter().cloned()));
    let signer = Arc::new(LocalSigner::new(keys.clone()));
    let publisher = Arc::new(Nip01Publisher);
    let delivery = Arc::new(StandardDeliveryPolicy::default());

    let base = FavaBuilder::default()
        .event_cache(event_cache)
        .query_evaluator(evaluator)
        .subscription_planner(sub_planner)
        .transport(transport)
        .router(router)
        .signer(signer)
        .publisher(publisher)
        .delivery_policy(delivery);

    if let Some(path) = store_path {
        let store = RedbWriteStore::open(path)
            .map_err(|e| store::opening_refused(e, path))?;
        base.write_store(Arc::new(store))
    } else {
        base.write_store(Arc::new(MemoryWriteStore::default()))
    }
    .build()
    .map_err(|e| anyhow::anyhow!("fava build failed: {e}"))
}

fn local_relay_hosts<'a>(relays: impl IntoIterator<Item = &'a RelayUrl>) -> Vec<String> {
    relays
        .into_iter()
        .filter(|relay| relay.is_localhost() || relay.is_local_addr())
        .filter_map(|relay| match relay.host() {
            Some(url::Host::Domain(d)) => Some(d.to_string()),
            Some(url::Host::Ipv4(ip)) => Some(ip.to_string()),
            Some(url::Host::Ipv6(ip)) => Some(ip.to_string()),
            None => None,
        })
        .collect()
}

#[cfg(test)]
mod tests;
