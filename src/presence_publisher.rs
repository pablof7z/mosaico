//! Latest-per-pubkey, non-blocking publication of reconciled presence effects.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

use nostr::Keys;
use tokio::sync::mpsc;

use crate::daemon::server::RuntimeSnapshot;
use crate::domain::{DomainEvent, Status};
use crate::fabric::provider::{ConfirmedGroupScope, Nip29Provider};
use crate::reconcile::{StatusEffect, StatusOutcome, StatusReconciler};
use crate::state::Store;

const PUBLISH_SIGNAL_CAPACITY: usize = 1;

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
        runtime: Arc<RwLock<Arc<RuntimeSnapshot>>>,
        store: Arc<Mutex<Store>>,
    ) -> PresencePublisher {
        let pending = Arc::new(Mutex::new(PendingPublishJobs::default()));
        let (signal_tx, signal_rx) = mpsc::channel(PUBLISH_SIGNAL_CAPACITY);
        spawn_publish_worker(signal_rx, pending.clone(), move |job| {
            let runtime = runtime.clone();
            let store = store.clone();
            async move {
                let snapshot = runtime
                    .read()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .clone();
                let event_ids = apply_status_effects(
                    &job.outcome,
                    &snapshot.provider,
                    &job.keys,
                    &job.trigger,
                    job.confirmed_scope.as_ref(),
                )
                .await;
                record_status_receipt(&store, &job.outcome, &event_ids);
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

async fn apply_status_effects(
    outcome: &StatusOutcome,
    provider: &Nip29Provider,
    keys: &Keys,
    trigger: &str,
    confirmed_scope: Option<&ConfirmedGroupScope>,
) -> Vec<String> {
    let mut ids = Vec::new();
    for effect in &outcome.effects {
        let status = match effect {
            StatusEffect::Publish { status, .. } | StatusEffect::Expire { status } => status,
        };
        let source_ref = format!(
            "status/{}#rev:{}:{trigger}",
            status.agent.pubkey, outcome.revision
        );
        if let Some(id) =
            enqueue_status(provider, keys, status.clone(), source_ref, confirmed_scope).await
        {
            ids.push(id);
        }
    }
    ids
}

fn record_status_receipt(store: &Mutex<Store>, outcome: &StatusOutcome, event_ids: &[String]) {
    let Some(artifact_ref) = event_ids.first().cloned() else {
        return;
    };
    let effects = outcome
        .effects
        .iter()
        .map(|effect| match effect {
            StatusEffect::Publish { reason, .. } => reason.as_str(),
            StatusEffect::Expire { .. } => "expire",
        })
        .collect::<Vec<_>>();
    let changed_summary = serde_json::json!({
        "pubkey": outcome.pubkey,
        "effects": effects,
    })
    .to_string();
    let row = crate::state::receipts::NewReceipt {
        surface: "status".into(),
        transaction_id: outcome.revision as i64,
        revision: outcome.revision as i64,
        changed_summary,
        commands: serde_json::to_string(&effects).unwrap_or_else(|_| "[]".into()),
        artifact_ref: Some(artifact_ref),
        created_at: crate::instrument::now_millis(),
    };
    crate::instrument::record_receipt(&store.lock().expect("store mutex poisoned"), row);
}

async fn enqueue_status(
    provider: &Nip29Provider,
    keys: &Keys,
    status: Status,
    source_ref: String,
    confirmed_scope: Option<&ConfirmedGroupScope>,
) -> Option<String> {
    let result = match confirmed_scope {
        Some(scope) => {
            provider
                .enqueue_status_after_confirmed_scope(&status, keys, scope)
                .await
        }
        None => provider.enqueue(&DomainEvent::Status(status), keys).await,
    };
    match result {
        Ok(event_id) => {
            tracing::debug!(event_id = %event_id.to_hex(), source_ref, "status accepted by NMP");
            Some(event_id.to_hex())
        }
        Err(error) => {
            tracing::error!(
                error = %format!("{error:#}"),
                source_ref,
                "status submission to NMP failed"
            );
            None
        }
    }
}
