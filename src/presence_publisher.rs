//! Latest-per-pubkey, non-blocking publication of reconciled presence effects.

use std::collections::{BTreeMap, VecDeque};
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

#[derive(Clone)]
pub(crate) struct PresencePublisher {
    signal_tx: mpsc::Sender<()>,
    pending: Arc<Mutex<PendingPublishJobs>>,
}

impl PresencePublisher {
    pub(crate) fn spawn(
        provider: Arc<RwLock<Arc<Nip29Provider>>>,
        store: Arc<Mutex<Store>>,
    ) -> PresencePublisher {
        let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
        let (signal_tx, signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
        let publish_pending = pending.clone();
        spawn_publish_worker(signal_rx, pending.clone(), move |job| {
            let provider = provider.clone();
            let store = store.clone();
            let pending = publish_pending.clone();
            async move {
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
            }
        });
        PresencePublisher { signal_tx, pending }
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

fn spawn_publish_worker<F, Fut>(
    mut signal_rx: mpsc::Receiver<()>,
    pending: Arc<Mutex<PendingPublishJobs>>,
    publish: F,
) where
    F: Fn(PublishJob) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        while signal_rx.recv().await.is_some() {
            loop {
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
