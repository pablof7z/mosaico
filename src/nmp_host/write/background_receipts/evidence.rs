use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

use serde::Serialize;

/// One NMP publish-queue fact, as Mosaico files it.
///
/// Every name here is NMP's own — there is no Mosaico taxonomy left to keep in
/// sync. `AuthFailed` stays distinct from `Rejected` because NMP keeps them
/// distinct: a local policy or signer refusal is not a relay refusing the
/// event, and collapsing them is the loss mosaico#745 recorded.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackgroundWriteTerminalStatus {
    Acked,
    Cancelled,
    SignerRefused,
    Rejected,
    AuthFailed,
    GaveUp,
    /// Local persistence refused a durable fact for this write. NMP emits it
    /// as a fact AND latches it on the queue entry; Mosaico files it here and
    /// a later relay ack never clears it.
    PersistenceStalled,
    /// The store answered acceptance with a semantic no — a stale replaceable
    /// base, a tombstone, an already-expired event. Permanently failed, with
    /// the payload intact in the queue.
    Refused,
    /// Routing finished and named zero relays. There is nowhere to publish.
    NoDestination,
    SubmissionFailed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BackgroundWriteGapStatus {
    CapacityFull,
    ObserverClosed,
    ReceiptTimeout,
    ReceiptDisconnected,
    ReceiptLagged,
    Shutdown,
    WorkerLost,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BackgroundWriteTerminalEvidence {
    pub(crate) operation: String,
    pub(crate) source_ref: String,
    pub(crate) target: String,
    pub(crate) status: BackgroundWriteTerminalStatus,
    pub(crate) detail: String,
    pub(crate) observed_at: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct BackgroundWriteGapEvidence {
    pub(crate) operation: String,
    pub(crate) source_ref: String,
    pub(crate) target: String,
    pub(crate) status: BackgroundWriteGapStatus,
    pub(crate) detail: String,
    pub(crate) observed_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct BackgroundWriteSnapshot {
    pub(crate) pending: usize,
    pub(crate) capacity: usize,
    pub(crate) admission_open: bool,
    pub(crate) workers: usize,
    pub(crate) configured_workers: usize,
    pub(crate) last_success: Option<BackgroundWriteTerminalEvidence>,
    pub(crate) last_failure: Option<BackgroundWriteTerminalEvidence>,
    pub(crate) last_gap: Option<BackgroundWriteGapEvidence>,
    pub(crate) recent_failures: Vec<BackgroundWriteTerminalEvidence>,
}

const FAILURE_HISTORY_CAPACITY: usize = 32;

#[derive(Default)]
struct State {
    last_success: Option<BackgroundWriteTerminalEvidence>,
    last_failure: Option<BackgroundWriteTerminalEvidence>,
    last_gap: Option<BackgroundWriteGapEvidence>,
    recent_failures: VecDeque<BackgroundWriteTerminalEvidence>,
}

#[derive(Default)]
pub(super) struct Evidence {
    state: Mutex<State>,
    pub(super) changed: Condvar,
}

impl Evidence {
    pub(super) fn success(&self, operation: &str, source_ref: &str, target: &str, detail: String) {
        let evidence = BackgroundWriteTerminalEvidence {
            operation: operation.into(),
            source_ref: source_ref.into(),
            target: target.into(),
            status: BackgroundWriteTerminalStatus::Acked,
            detail,
            observed_at: crate::util::now_secs(),
        };
        tracing::debug!(
            operation,
            source_ref,
            target,
            status = ?evidence.status,
            "background NMP write terminal success observed"
        );
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .last_success = Some(evidence);
        self.changed.notify_all();
    }

    pub(super) fn failure(
        &self,
        operation: &str,
        source_ref: &str,
        target: &str,
        status: BackgroundWriteTerminalStatus,
        detail: String,
    ) {
        let evidence = BackgroundWriteTerminalEvidence {
            operation: operation.into(),
            source_ref: source_ref.into(),
            target: target.into(),
            status,
            detail,
            observed_at: crate::util::now_secs(),
        };
        tracing::error!(
            operation,
            source_ref,
            target,
            status = ?evidence.status,
            error = %evidence.detail,
            "background NMP write failure observed"
        );
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.recent_failures.push_back(evidence.clone());
        if state.recent_failures.len() > FAILURE_HISTORY_CAPACITY {
            state.recent_failures.pop_front();
        }
        let preserves_submission_cause = state.last_failure.as_ref().is_some_and(|previous| {
            previous.operation == evidence.operation
                && previous.source_ref == evidence.source_ref
                && previous.status == BackgroundWriteTerminalStatus::SubmissionFailed
                && evidence.status != BackgroundWriteTerminalStatus::SubmissionFailed
        });
        if !preserves_submission_cause {
            state.last_failure = Some(evidence);
        }
        drop(state);
        self.changed.notify_all();
    }

    pub(super) fn gap(
        &self,
        operation: &str,
        source_ref: &str,
        target: &str,
        status: BackgroundWriteGapStatus,
        detail: String,
    ) {
        let evidence = BackgroundWriteGapEvidence {
            operation: operation.into(),
            source_ref: source_ref.into(),
            target: target.into(),
            status,
            detail,
            observed_at: crate::util::now_secs(),
        };
        tracing::warn!(
            operation,
            source_ref,
            target,
            status = ?evidence.status,
            detail = %evidence.detail,
            "background NMP receipt observation gap"
        );
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .last_gap = Some(evidence);
        self.changed.notify_all();
    }

    pub(super) fn snapshot(
        &self,
        pending: usize,
        capacity: usize,
        admission_open: bool,
        workers: usize,
        configured_workers: usize,
    ) -> BackgroundWriteSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        BackgroundWriteSnapshot {
            pending,
            capacity,
            admission_open,
            workers,
            configured_workers,
            last_success: state.last_success.clone(),
            last_failure: state.last_failure.clone(),
            last_gap: state.last_gap.clone(),
            recent_failures: state.recent_failures.iter().cloned().collect(),
        }
    }

    #[cfg(test)]
    pub(super) fn wait_for_failure(&self, source_ref: &str, timeout: std::time::Duration) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        drop(
            self.changed
                .wait_timeout_while(state, timeout, |state| {
                    state
                        .last_failure
                        .as_ref()
                        .is_none_or(|failure| failure.source_ref != source_ref)
                })
                .unwrap()
                .0,
        );
    }
}
