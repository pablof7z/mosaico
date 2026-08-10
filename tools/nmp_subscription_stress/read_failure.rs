use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{ensure, Result};
use nmp::mechanism::core::RelayAdmissionPolicy;
use nmp::mechanism::runtime::{EngineThread, EngineThreadError, ObservationOwnershipCensus};
use nmp_store::{
    AcceptOutcome, AcceptWrite, CompensateOutcome, CompensationReason, CoverageInterval,
    CoverageKey, EventStore, GcReport, GcRetentionSet, InsertOutcome, IntentId, MemoryStore,
    PersistenceError, PersistenceFault, PromoteOutcome, PublishQueueAttempt, PublishQueueIntent,
    PublishQueueReceipt, PublishQueueRouteRevision, RefuseReason, RelayObserved,
    RemoveQueueEntryOutcome, RetractReason, StoredEvent,
};
use nmp_transport::PoolConfig;
use nostr::{Event, EventId, PublicKey, RelayUrl, Timestamp};

use crate::measure::{elapsed_since, process_cpu_time, Metric, Samples};
use crate::workload::Workload;

pub(crate) fn run(workload: &Workload) -> Result<Metric> {
    let failure = Arc::new(AtomicBool::new(true));
    let store = FailOnceReadStore::new(failure.clone());
    let (thread, handle) = EngineThread::spawn(
        store,
        8,
        PoolConfig::default(),
        RelayAdmissionPolicy::default(),
    )?;
    let query = workload.profile_query(0)?;
    let started = Instant::now();
    let cpu_started = process_cpu_time();
    let refused = handle.subscribe(query.clone());
    ensure!(
        matches!(refused, Err(EngineThreadError::ObservationUnavailable { ref reason })
            if reason.contains("injected canonical read failure")),
        "store read failure did not surface as the typed observation refusal"
    );
    ensure!(
        handle.observation_ownership_census() == ObservationOwnershipCensus::default(),
        "refused observation escaped ownership"
    );
    ensure!(!failure.load(Ordering::SeqCst));

    let (healthy, rows) = handle.subscribe(query)?;
    rows.recv_timeout(Duration::from_secs(1)).map_err(|error| {
        anyhow::anyhow!("healthy reopen did not produce its initial frame: {error}")
    })?;
    handle.unsubscribe(healthy);
    ensure!(
        handle.observation_ownership_census() == ObservationOwnershipCensus::default(),
        "healthy reopen did not tear down"
    );
    handle.shutdown();
    thread.join();
    let (elapsed, cpu) = elapsed_since(started, cpu_started);
    let mut samples = Samples::default();
    samples.push(elapsed);
    Ok(Metric::new(
        "runtime_control",
        "store_read_failure",
        "typed_refusal_then_recovery",
        elapsed,
        samples,
    )
    .cpu(cpu)
    .count("typed_refusals", 1)
    .count("escaped_owners_after_refusal", 0)
    .count("healthy_reopens", 1)
    .count("final_ownership_census", 0)
    .contract_status(true)
    .note("one deterministic canonical read failure refuses without a handle; the same runtime then reopens and tears down"))
}

struct FailOnceReadStore {
    inner: MemoryStore,
    fail: Arc<AtomicBool>,
}

impl FailOnceReadStore {
    fn new(fail: Arc<AtomicBool>) -> Self {
        Self {
            inner: MemoryStore::new(),
            fail,
        }
    }
}

impl EventStore for FailOnceReadStore {
    fn query(&self, filter: &nostr::Filter) -> Result<Vec<StoredEvent>, PersistenceError> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(PersistenceError::new(
                PersistenceFault::Io,
                "injected canonical read failure",
            ));
        }
        self.inner.query(filter)
    }

    fn insert(
        &mut self,
        event: Event,
        from: RelayObserved,
    ) -> Result<InsertOutcome, PersistenceError> {
        self.inner.insert(event, from)
    }

    fn remove(
        &mut self,
        id: EventId,
        reason: RetractReason,
    ) -> Result<Option<StoredEvent>, PersistenceError> {
        self.inner.remove(id, reason)
    }

    fn expire_due(&mut self, now: Timestamp) -> Result<Vec<StoredEvent>, PersistenceError> {
        self.inner.expire_due(now)
    }

    fn next_expiration(&self) -> Result<Option<Timestamp>, PersistenceError> {
        self.inner.next_expiration()
    }

    fn record_coverage(
        &mut self,
        claims: &[(nmp_grammar::ContextualAtom, RelayUrl, CoverageInterval)],
    ) -> Result<(), PersistenceError> {
        self.inner.record_coverage(claims)
    }

    fn get_coverage(
        &self,
        key: CoverageKey,
        relay: &RelayUrl,
    ) -> Result<Option<CoverageInterval>, PersistenceError> {
        self.inner.get_coverage(key, relay)
    }

    fn gc(&mut self, claims: &GcRetentionSet) -> Result<GcReport, PersistenceError> {
        self.inner.gc(claims)
    }

    fn accept_write(&mut self, accept: AcceptWrite) -> Result<AcceptOutcome, PersistenceError> {
        self.inner.accept_write(accept)
    }

    fn enumerate_publish_queue_receipts(
        &self,
    ) -> Result<Vec<PublishQueueReceipt>, PersistenceError> {
        self.inner.enumerate_publish_queue_receipts()
    }

    fn remove_publish_queue_entry(
        &mut self,
        receipt_id: u64,
    ) -> Result<RemoveQueueEntryOutcome, PersistenceError> {
        self.inner.remove_publish_queue_entry(receipt_id)
    }

    fn accept_refused(
        &mut self,
        frozen_id: EventId,
        expected_pubkey: PublicKey,
        reason: RefuseReason,
    ) -> Result<u64, PersistenceError> {
        self.inner
            .accept_refused(frozen_id, expected_pubkey, reason)
    }

    fn promote_signed(
        &mut self,
        intent_id: IntentId,
        verified: nmp_store::VerifiedSignature,
    ) -> Result<PromoteOutcome, PersistenceError> {
        self.inner.promote_signed(intent_id, verified)
    }

    fn compensate_write(
        &mut self,
        intent_id: IntentId,
    ) -> Result<CompensateOutcome, PersistenceError> {
        self.inner.compensate_write(intent_id)
    }

    fn compensate_write_with_state(
        &mut self,
        intent_id: IntentId,
        reason: CompensationReason,
    ) -> Result<CompensateOutcome, PersistenceError> {
        self.inner.compensate_write_with_state(intent_id, reason)
    }

    fn recover_publish_queue(&self) -> Result<Vec<PublishQueueIntent>, PersistenceError> {
        self.inner.recover_publish_queue()
    }

    fn reattach_receipt(
        &self,
        receipt_id: u64,
    ) -> Result<Option<PublishQueueReceipt>, PersistenceError> {
        self.inner.reattach_receipt(receipt_id)
    }

    fn lookup_correlation(&self, token: &str) -> Result<Option<u64>, PersistenceError> {
        self.inner.lookup_correlation(token)
    }

    fn record_route_revision(
        &mut self,
        intent_id: IntentId,
        relays: BTreeSet<RelayUrl>,
    ) -> Result<PublishQueueRouteRevision, PersistenceError> {
        self.inner.record_route_revision(intent_id, relays)
    }

    fn recover_route_revisions(
        &self,
        intent_id: IntentId,
    ) -> Result<Vec<PublishQueueRouteRevision>, PersistenceError> {
        self.inner.recover_route_revisions(intent_id)
    }

    fn recover_attempts(
        &self,
        intent_id: IntentId,
    ) -> Result<Vec<PublishQueueAttempt>, PersistenceError> {
        self.inner.recover_attempts(intent_id)
    }
}

#[cfg(test)]
#[path = "read_failure/tests.rs"]
mod tests;
