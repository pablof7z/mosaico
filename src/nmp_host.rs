//! NMP-backed Nostr acquisition and durable publication.
//!
//! NMP owns relay planning, subscription lifecycle, canonical wire-event
//! deduplication, current event state, and acquisition evidence. Mosaico keeps
//! only process-local presentation projections and genuinely local durable
//! facts. NMP also owns every durable write intent, route, receipt, and bounded
//! retry. Shared reads are public because Mosaico-created groups are deliberately
//! public; writes authenticate as the exact event author. The provider supplies
//! product policy and exact host authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use nmp::{AccessContext, Engine, EngineConfig, ObservationCancel, RelayUrl};
use nostr::Keys;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
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
use auth::IdentityRegistration;

const VIEW_TRANSITION_QUEUE_CAPACITY: usize = 2048;
const MAX_LOCAL_IDENTITIES: usize = 4096;

struct ActiveObservation {
    cancel: ObservationCancel,
}

pub(crate) struct NmpHost {
    engine: Engine,
    relays: BTreeSet<RelayUrl>,
    profile_relays: BTreeSet<RelayUrl>,
    identities: Mutex<BTreeMap<nostr::PublicKey, IdentityRegistration>>,
    signing: Mutex<()>,
    subscriptions: Mutex<BTreeMap<String, ActiveObservation>>,
    next_observation_generation: Mutex<u64>,
    views: Arc<NmpViews>,
    transition_tx: Mutex<Option<mpsc::Sender<RowTransition>>>,
    transition_rx: Mutex<Option<mpsc::Receiver<RowTransition>>>,
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
        let (transition_tx, transition_rx) = mpsc::channel(VIEW_TRANSITION_QUEUE_CAPACITY);
        let host = Self {
            engine,
            relays: parsed,
            profile_relays,
            identities: Mutex::new(BTreeMap::new()),
            signing: Mutex::new(()),
            subscriptions: Mutex::new(BTreeMap::new()),
            next_observation_generation: Mutex::new(0),
            views: Arc::new(NmpViews::default()),
            transition_tx: Mutex::new(Some(transition_tx)),
            transition_rx: Mutex::new(Some(transition_rx)),
            #[cfg(test)]
            test_io: test_io::TestIo::default(),
        };
        host.ensure_identity(backend_keys)
            .context("registering backend NIP-42 identity")?;
        Ok(host)
    }

    /// Take the one lossless stream feeding Mosaico's product side effects.
    /// A bounded channel deliberately backpressures observation drains instead
    /// of dropping NMP transitions under a relay burst.
    pub(crate) fn views(&self) -> &NmpViews {
        &self.views
    }

    pub(crate) fn views_handle(&self) -> Arc<NmpViews> {
        self.views.clone()
    }

    pub(crate) fn take_view_transitions(&self) -> Result<mpsc::Receiver<RowTransition>> {
        self.transition_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .context("NMP view transition stream is already owned")
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
        for (_, active) in subscriptions {
            active.cancel.cancel();
        }
        self.transition_tx
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
        std::thread::Builder::new()
            .name(format!("nmp-{id}"))
            .spawn(move || {
                while let Ok(frame) = subscription.recv() {
                    let transition = views.apply_frame(
                        &observation_id,
                        generation,
                        frame.deltas,
                        frame.evidence,
                    );
                    if !transition.is_empty() && transitions.blocking_send(transition).is_err() {
                        return;
                    }
                }
                let transition = views.close_observation(&observation_id, generation);
                if !transition.is_empty() {
                    let _ = transitions.blocking_send(transition);
                }
            })
            .with_context(|| format!("starting NMP observation drain {id}"))?;
        let previous = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .insert(id.to_string(), ActiveObservation { cancel });
        if let Some(previous) = previous {
            previous.cancel.cancel();
        }
        Ok(())
    }

    fn close_subscription(&self, id: &str) {
        if let Some(active) = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id)
        {
            active.cancel.cancel();
        }
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
