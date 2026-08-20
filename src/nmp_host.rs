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
use nmp::{Engine, EngineConfig, ObservationCancel, PublicKey, RelayUrl};
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

mod profile_feed;
pub(crate) use profile_feed::ProfileFeed;
#[cfg(test)]
mod test_relay;
#[cfg(test)]
pub(crate) use test_relay::PlainRelay;

const VIEW_TRANSITION_QUEUE_CAPACITY: usize = 2048;
const MAX_LOCAL_IDENTITIES: usize = 4096;

/// Bounded newest-first window per observation: NMP delivers the recent
/// `OBSERVATION_WINDOW_INITIAL` rows on boot and grows up to
/// `OBSERVATION_WINDOW_MAX` on explicit `request_rows` (the on-demand path for
/// agent tools reading older history). This bounds the boot read and the
/// retained row set — no O(history) mirror rebuild (#837). The max matches
/// `RecentRows`' per-channel cap so the retained window and the delivered
/// window agree.
const OBSERVATION_WINDOW_INITIAL: usize = 500;
pub(crate) const OBSERVATION_WINDOW_MAX: usize = 10_000;

struct ActiveObservation {
    cancel: ObservationCancel,
    /// Cloneable growth capability for a windowed observation. `None` for an
    /// unbounded observation. `request_rows` grows the window for on-demand
    /// older-history backfill (mosaico#837).
    window: Option<nmp::WindowHandle>,
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
        // The backend key is the engine's default current account: `sign_event`
        // signs as whichever account is current, and Mosaico's sign path
        // temporarily swaps in the target author before restoring this one.
        host.engine
            .make_current_account(backend_keys.public_key())
            .context("selecting backend NMP account")?;
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

    /// Open a caller-owned NMP observation. Dropping the returned value closes
    /// it, making this suitable for precise, short-lived correlation queries.
    pub(crate) fn observe(&self, query: &SubscriptionQuery) -> Result<nmp::Subscription> {
        self.observe_with_access(query, None)
    }

    fn observe_with_access(
        &self,
        query: &SubscriptionQuery,
        authenticate_as: Option<PublicKey>,
    ) -> Result<nmp::Subscription> {
        // Bounded newest-first window: NMP delivers only the recent INITIAL
        // rows on boot and grows up to MAX on explicit `request_rows` (the
        // on-demand path for agent tools reading older history). This bounds
        // the boot read and the retained row set — no O(history) mirror
        // rebuild (#837). It is the option-B contract: recent activity is
        // sync-readable from the retained window; older rows are fetched on
        // demand, not mirrored.
        let window = nmp::Window::Expandable {
            initial: std::num::NonZeroUsize::new(OBSERVATION_WINDOW_INITIAL)
                .expect("window initial is nonzero"),
            max: std::num::NonZeroUsize::new(OBSERVATION_WINDOW_MAX)
                .expect("window max is nonzero"),
        };
        self.engine
            .observe(self.live_query(query, authenticate_as)?, Some(window))
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
        let window = subscription.window_handle();
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
            .insert(id.to_string(), ActiveObservation { cancel, window });
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

    /// Monotonically raise the windowed observation's row target to at least
    /// `at_least`, clamped to the observation's declared max. Idempotent; a
    /// value at or below the current target is a no-op. The broad `#h`
    /// observation's window is total across all channels (not per `#h`), so this
    /// backfills older history for every covered channel at once (mosaico#837).
    /// Growth arrives as new rows in subsequent frames — the caller re-reads to
    /// see them. Returns an error for an unwindowed or unknown observation.
    pub(crate) fn request_rows(&self, id: &str, at_least: usize) -> Result<()> {
        let active = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .get(id)
            .map(|active| active.window.clone());
        match active {
            Some(Some(handle)) => handle
                .request_rows(at_least)
                .with_context(|| format!("requesting {at_least} rows for {id}")),
            Some(None) => anyhow::bail!("observation {id} is not windowed"),
            None => anyhow::bail!("observation {id} is not open"),
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

#[cfg(test)]
mod tests;
