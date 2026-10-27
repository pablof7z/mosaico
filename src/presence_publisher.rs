//! Latest-per-pubkey, non-blocking publication of reconciled presence effects.

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};

use nostr::key::Keys;
use tokio::sync::mpsc;

use crate::fabric::provider::{ConfirmedGroupScope, Nip29Provider};
use crate::reconcile::{StatusOutcome, StatusReconciler};
use crate::state::Store;

#[cfg(test)]
use crate::{domain::Status, reconcile::StatusEffect};

const PUBLISH_SIGNAL_CAPACITY: usize = 1;

#[path = "presence_publisher/publication.rs"]
mod publication;

#[cfg(test)]
#[path = "presence_publisher/tests.rs"]
mod tests;

pub(crate) struct DriveMeta<'a> {
    pub trigger: &'a str,
    pub confirmed_scope: Option<ConfirmedGroupScope>,
}

struct PublishJob {
    outcome: StatusOutcome,
    keys: Keys,
    trigger: String,
    confirmed_scope: Option<ConfirmedGroupScope>,
}

#[derive(Default)]
struct PendingPublishJobs {
    order: VecDeque<String>,
    jobs: BTreeMap<String, PublishJob>,
}

impl PendingPublishJobs {
    fn push(&mut self, job: PublishJob) {
        let pubkey = job
            .outcome
            .pubkey
            .clone()
            .expect("a non-empty status outcome must name its pubkey");
        if self.jobs.insert(pubkey.clone(), job).is_none() {
            self.order.push_back(pubkey);
        }
    }

    /// Drop a queued job that an awaited publication of `revision` replaces.
    fn discard_superseded(&mut self, pubkey: &str, revision: u64) {
        if self
            .jobs
            .get(pubkey)
            .is_some_and(|job| job.outcome.revision <= revision)
        {
            self.jobs.remove(pubkey);
            self.order.retain(|queued| queued != pubkey);
        }
    }

    fn pop_front(&mut self) -> Option<PublishJob> {
        let pubkey = self.order.pop_front()?;
        self.jobs.remove(&pubkey)
    }

    fn clear(&mut self) {
        self.order.clear();
        self.jobs.clear();
    }

    fn supersedes(&self, pubkey: &str, revision: u64) -> bool {
        self.jobs
            .get(pubkey)
            .is_some_and(|job| job.outcome.revision > revision)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.jobs.len()
    }
}

type PublishFn = Arc<dyn Fn(PublishJob) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Clone)]
pub(crate) struct PresencePublisher {
    signal_tx: mpsc::Sender<()>,
    pending: Arc<Mutex<PendingPublishJobs>>,
    /// Serializes relay publication between the background worker and
    /// awaited publications, so neither can land a stale state after the other.
    lane: Arc<tokio::sync::Mutex<()>>,
    publish: PublishFn,
}

impl PresencePublisher {
    pub(crate) fn spawn(
        provider: Arc<RwLock<Arc<Nip29Provider>>>,
        store: Arc<Mutex<Store>>,
    ) -> PresencePublisher {
        let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
        let publish_pending = pending.clone();
        Self::start(
            pending,
            Arc::new(move |job: PublishJob| {
                let provider = provider.clone();
                let store = store.clone();
                let pending = publish_pending.clone();
                Box::pin(async move {
                    let provider = provider
                        .read()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .clone();
                    let event_ids = publication::apply_status_effects(
                        &job.outcome,
                        &provider,
                        &job.keys,
                        &job.trigger,
                        job.confirmed_scope.as_ref(),
                        &pending,
                    )
                    .await;
                    publication::record_status_receipt(&store, &job.outcome, &event_ids);
                })
            }),
        )
    }

    fn start(pending: Arc<Mutex<PendingPublishJobs>>, publish: PublishFn) -> PresencePublisher {
        let (signal_tx, signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
        let lane = Arc::new(tokio::sync::Mutex::new(()));
        spawn_publish_worker(signal_rx, pending.clone(), lane.clone(), publish.clone());
        PresencePublisher {
            signal_tx,
            pending,
            lane,
            publish,
        }
    }

    /// Publish `outcome` now and return once the relay has answered.
    ///
    /// For effects that must land before a following relay mutation (an
    /// expiry that only a current member may still write). It takes the same
    /// publication lane as the worker and replaces any older queued state.
    pub(crate) async fn publish_now(
        &self,
        outcome: StatusOutcome,
        keys: &Keys,
        trigger: &str,
        confirmed_scope: Option<ConfirmedGroupScope>,
    ) {
        let Some(pubkey) = outcome.pubkey.clone() else {
            return;
        };
        if outcome.effects.is_empty() {
            return;
        }
        let _lane = self.lane.lock().await;
        self.pending
            .lock()
            .expect("presence publish queue poisoned")
            .discard_superseded(&pubkey, outcome.revision);
        (self.publish)(PublishJob {
            outcome,
            keys: keys.clone(),
            trigger: trigger.to_string(),
            confirmed_scope,
        })
        .await;
    }

    fn submit(
        &self,
        outcome: StatusOutcome,
        keys: &Keys,
        trigger: &str,
        confirmed_scope: Option<ConfirmedGroupScope>,
    ) {
        if outcome.effects.is_empty() {
            return;
        }
        self.pending
            .lock()
            .expect("presence publish queue poisoned")
            .push(PublishJob {
                outcome,
                keys: keys.clone(),
                trigger: trigger.to_string(),
                confirmed_scope,
            });
        match self.signal_tx.try_send(()) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
            Err(error @ mpsc::error::TrySendError::Closed(())) => {
                self.pending
                    .lock()
                    .expect("presence publish queue poisoned")
                    .clear();
                tracing::warn!(%error, trigger, "presence publish worker is closed");
            }
        }
    }
}

fn spawn_publish_worker(
    mut signal_rx: mpsc::Receiver<()>,
    pending: Arc<Mutex<PendingPublishJobs>>,
    lane: Arc<tokio::sync::Mutex<()>>,
    publish: PublishFn,
) {
    tokio::spawn(async move {
        while signal_rx.recv().await.is_some() {
            loop {
                let _lane = lane.lock().await;
                let job = pending
                    .lock()
                    .expect("presence publish queue poisoned")
                    .pop_front();
                let Some(job) = job else {
                    break;
                };
                publish(job).await;
            }
        }
    });
}

pub(crate) fn drive(
    status: &Mutex<StatusReconciler>,
    publisher: &PresencePublisher,
    keys: &Keys,
    meta: DriveMeta<'_>,
    f: impl FnOnce(&mut StatusReconciler) -> StatusOutcome,
) {
    let outcome = {
        let mut policy = status.lock().expect("status policy poisoned");
        f(&mut policy)
    };
    publisher.submit(outcome, keys, meta.trigger, meta.confirmed_scope);
}
