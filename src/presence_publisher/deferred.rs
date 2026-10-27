//! Status publications the local publish gate could not yet verify.
//!
//! Right after a daemon restart the group view is empty until the relay's
//! group records arrive, so the gate refuses a session's Opened status and
//! the next attempt would only come with the lease renewal. A refused job is
//! parked (latest per pubkey) and requeued when the decoded group state
//! changes.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use tokio::sync::mpsc;

use super::{PendingPublishJobs, PublishJob};
use crate::fabric::provider::Nip29Provider;

/// Re-subscribe at least this often, so a provider replaced by configuration
/// reload is followed even when the old view never changes again.
const RESUBSCRIBE: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct DeferredJobs(BTreeMap<String, PublishJob>);

impl DeferredJobs {
    pub(super) fn park(&mut self, job: PublishJob) {
        let Some(pubkey) = job.outcome.pubkey.clone() else {
            return;
        };
        if self
            .0
            .get(&pubkey)
            .is_none_or(|parked| parked.outcome.revision <= job.outcome.revision)
        {
            self.0.insert(pubkey, job);
        }
    }

    /// Record how a job's publication ended: park it if the publish gate
    /// refused it, otherwise retire any older parked state for its pubkey so a
    /// later group change cannot requeue a status this one replaced.
    pub(super) fn settle(&mut self, job: PublishJob, deferred: bool) {
        if deferred {
            self.park(job);
        } else if let Some(pubkey) = job.outcome.pubkey.as_deref() {
            self.discard_superseded(pubkey, job.outcome.revision);
        }
    }

    pub(super) fn discard_superseded(&mut self, pubkey: &str, revision: u64) {
        if self
            .0
            .get(pubkey)
            .is_some_and(|parked| parked.outcome.revision <= revision)
        {
            self.0.remove(pubkey);
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.0.len()
    }

    /// Move every parked job back into `pending`, unless a job at least as
    /// new is already queued for that pubkey. Returns how many were requeued.
    pub(super) fn requeue_into(&mut self, pending: &mut PendingPublishJobs) -> usize {
        let mut requeued = 0;
        for (pubkey, job) in std::mem::take(&mut self.0) {
            if pending
                .jobs
                .get(&pubkey)
                .is_some_and(|queued| queued.outcome.revision >= job.outcome.revision)
            {
                continue;
            }
            pending.push(job);
            requeued += 1;
        }
        requeued
    }
}

pub(super) fn spawn_requeue_on_group_change(
    provider: Arc<RwLock<Arc<Nip29Provider>>>,
    deferred: Arc<Mutex<DeferredJobs>>,
    pending: Arc<Mutex<PendingPublishJobs>>,
    signal_tx: mpsc::Sender<()>,
) {
    tokio::spawn(async move {
        loop {
            let mut changes = provider
                .read()
                .unwrap_or_else(|poison| poison.into_inner())
                .subscribe_group_changes();
            match tokio::time::timeout(RESUBSCRIBE, changes.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => {
                    // The view was replaced; follow the current provider.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(_) => continue,
            }
            let requeued = {
                let mut pending = pending.lock().expect("presence publish queue poisoned");
                deferred
                    .lock()
                    .expect("deferred presence queue poisoned")
                    .requeue_into(&mut pending)
            };
            if requeued > 0 && signal_tx.is_closed() {
                return;
            }
            if requeued > 0 {
                let _ = signal_tx.try_send(());
            }
        }
    });
}

#[cfg(test)]
#[path = "deferred/tests.rs"]
mod tests;
