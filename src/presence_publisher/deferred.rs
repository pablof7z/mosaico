//! Status publications the local publish gate could not yet verify.
//!
//! Right after a daemon restart the group view is empty until the relay's
//! group records arrive, so the gate refuses a session's Opened status and
//! the next attempt would only come with the lease renewal. A refused job is
//! parked (latest per pubkey) and requeued when the decoded group state
//! changes.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use super::{PendingPublishJobs, PresencePublisher, PublishJob};
use crate::fabric::provider::Nip29Provider;

/// Retry parked statuses at least this often even without a group change,
/// and follow a provider replaced by configuration reload.
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
    pub(super) fn settle(&mut self, mut job: PublishJob, deferred: bool) {
        if deferred {
            job.deferrals = job.deferrals.saturating_add(1);
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

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
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
    publisher: PresencePublisher,
) {
    let subscribe = move || {
        provider
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .subscribe_group_changes()
    };
    tokio::spawn(async move {
        // One long-lived receiver: a group change that lands while a requeue
        // is running stays pending for the next wait instead of being lost to
        // a fresh subscription.
        let mut changes = subscribe();
        loop {
            match tokio::time::timeout(RESUBSCRIBE, changes.changed()).await {
                Ok(Ok(())) => {}
                // The view was replaced (configuration reload), or nothing
                // changed for a while: follow the current provider and retry
                // anyway, so a parked status never waits on a missed wake.
                Ok(Err(_)) | Err(_) => changes = subscribe(),
            }
            if publisher.requeue_deferred().await && publisher.signal_tx.is_closed() {
                return;
            }
        }
    });
}

impl PresencePublisher {
    /// Requeue parked statuses and wake the worker. Holds the publication
    /// lane, so a newer status for the same pubkey that is publishing right
    /// now settles first (retiring the parked one) instead of being
    /// overwritten by it afterwards. Returns whether anything was requeued.
    pub(super) async fn requeue_deferred(&self) -> bool {
        // Nothing parked: never take the lane (an awaited publication may want it).
        if self
            .deferred
            .lock()
            .expect("deferred presence queue poisoned")
            .is_empty()
        {
            return false;
        }
        let _lane = self.lane.lock().await;
        let requeued = {
            let mut pending = self
                .pending
                .lock()
                .expect("presence publish queue poisoned");
            self.deferred
                .lock()
                .expect("deferred presence queue poisoned")
                .requeue_into(&mut pending)
        };
        if requeued > 0 {
            let _ = self.signal_tx.try_send(());
        }
        requeued > 0
    }
}

#[cfg(test)]
#[path = "deferred/tests.rs"]
mod tests;
