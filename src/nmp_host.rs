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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use anyhow::{Context, Result};
use nmp::{AccessContext, Engine, EngineConfig, RelayUrl};
use nostr::Keys;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
mod auth;
mod materialization;
mod query;
pub(crate) mod read;
mod scrub;
pub(crate) mod store;
#[cfg(feature = "stress-harness")]
mod stress;
#[cfg(test)]
mod test_io;
pub(crate) mod write;

use auth::IdentityRegistration;
use materialization::ActiveObservation;
pub(crate) use materialization::{relay_settled, scoped_evidence_json};
pub(crate) use materialization::{MaterializationBatch, MaterializationPhase};

const MATERIALIZATION_QUEUE_CAPACITY: usize = 2048;
const MAX_LOCAL_IDENTITIES: usize = 4096;

pub(crate) struct NmpHost {
    engine: Engine,
    relays: BTreeSet<RelayUrl>,
    profile_relays: BTreeSet<RelayUrl>,
    identities: Mutex<BTreeMap<nostr::PublicKey, IdentityRegistration>>,
    signing: Mutex<()>,
    subscriptions: Mutex<BTreeMap<String, ActiveObservation>>,
    observation_generations: Mutex<BTreeMap<String, u64>>,
    next_observation_generation: AtomicU64,
    materialization_tx: Mutex<Option<mpsc::Sender<MaterializationBatch>>>,
    materialization_rx: Mutex<Option<mpsc::Receiver<MaterializationBatch>>>,
    #[cfg(test)]
    test_io: test_io::TestIo,
}

impl NmpHost {
    /// Open the durable store and the engine over it.
    ///
    /// A store NMP refuses comes back as an error carrying the typed
    /// [`nmp::EngineError`] as its source, so a caller decides what to do from
    /// [`store::StoreCondition::of_open_error`] — never by reading the message.
    /// The distinction that matters is the one between a superseded schema
    /// epoch, which only discarding the store fixes, and every other refusal,
    /// which discarding the store makes permanently worse.
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
        let engine = Engine::new(config).map_err(store::opening_refused)?;
        let (materialization_tx, materialization_rx) =
            mpsc::channel(MATERIALIZATION_QUEUE_CAPACITY);
        let host = Self {
            engine,
            relays: parsed,
            profile_relays,
            identities: Mutex::new(BTreeMap::new()),
            signing: Mutex::new(()),
            subscriptions: Mutex::new(BTreeMap::new()),
            observation_generations: Mutex::new(BTreeMap::new()),
            next_observation_generation: AtomicU64::new(observation_generation_seed()),
            materialization_tx: Mutex::new(Some(materialization_tx)),
            materialization_rx: Mutex::new(Some(materialization_rx)),
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
        for (_, observation) in subscriptions {
            observation.cancel.cancel();
        }
        self.materialization_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.engine.shutdown();
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
        let generation = self
            .next_observation_generation
            .fetch_add(1, Ordering::Relaxed);
        self.observation_generations
            .lock()
            .expect("NMP observation-generation mutex poisoned")
            .insert(id.to_string(), generation);
        let observation_id = id.to_string();
        std::thread::Builder::new()
            .name(format!("nmp-{id}"))
            .spawn(move || {
                while let Ok(frame) = subscription.recv() {
                    let batch =
                        MaterializationBatch::from_frame(&observation_id, generation, &frame);
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
            .insert(id.to_string(), ActiveObservation { generation, cancel });
        if let Some(previous) = previous {
            previous.cancel.cancel();
        }
        Ok(())
    }

    fn close_subscription(&self, id: &str) {
        if let Some(observation) = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id)
        {
            observation.cancel.cancel();
            let materialization = self
                .materialization_tx
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone();
            let observation_id = id.to_string();
            std::thread::spawn(move || {
                if let Some(materialization) = materialization {
                    let _ = materialization.blocking_send(MaterializationBatch::closed(
                        observation_id,
                        observation.generation,
                    ));
                }
            });
        }
    }

    pub(crate) fn accepts_materialization(&self, batch: &MaterializationBatch) -> bool {
        let latest = self
            .observation_generations
            .lock()
            .expect("NMP observation-generation mutex poisoned")
            .get(&batch.observation_id)
            .copied();
        if latest != Some(batch.generation) {
            return false;
        }
        match batch.phase {
            MaterializationPhase::Frame => self
                .subscriptions
                .lock()
                .expect("NMP subscription mutex poisoned")
                .get(&batch.observation_id)
                .is_some_and(|active| active.generation == batch.generation),
            MaterializationPhase::Closed => !self
                .subscriptions
                .lock()
                .expect("NMP subscription mutex poisoned")
                .contains_key(&batch.observation_id),
        }
    }

    pub(crate) fn allocate_projection_generation(&self) -> u64 {
        self.next_observation_generation
            .fetch_add(1, Ordering::Relaxed)
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

fn observation_generation_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock predates Unix epoch")
        .as_nanos()
        .try_into()
        .expect("Unix nanosecond timestamp exceeds u64")
}

#[cfg(test)]
mod tests;
