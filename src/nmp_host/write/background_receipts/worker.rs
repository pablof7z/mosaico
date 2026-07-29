use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nmp::{FifoReceiver, FifoRecvTimeoutError, WriteStatus};

use super::admission::ReceiptSlot;
use super::evidence::{BackgroundWriteGapStatus, BackgroundWriteTerminalStatus, Evidence};

const SHUTDOWN_WAKE_CADENCE: Duration = Duration::from_millis(100);

pub(super) struct ReceiptJob {
    pub(super) receiver: FifoReceiver<WriteStatus>,
    pub(super) target: String,
    pub(super) deadline: Instant,
    pub(super) tracker: Arc<Tracker>,
    pub(super) _slot: ReceiptSlot,
}

struct TrackerState {
    remaining: usize,
    clean: bool,
}

pub(super) struct Tracker {
    operation: String,
    source_ref: String,
    allow_success: bool,
    state: Mutex<TrackerState>,
    evidence: Arc<Evidence>,
}

impl Tracker {
    pub(super) fn new(
        operation: &str,
        source_ref: &str,
        streams: usize,
        allow_success: bool,
        evidence: Arc<Evidence>,
    ) -> Self {
        Self {
            operation: operation.into(),
            source_ref: source_ref.into(),
            allow_success,
            state: Mutex::new(TrackerState {
                remaining: streams,
                clean: true,
            }),
            evidence,
        }
    }

    fn finish(&self, target: &str, outcome: StreamOutcome) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        match &outcome {
            StreamOutcome::Acked => {}
            StreamOutcome::Failure(status, detail) => {
                state.clean = false;
                self.evidence.failure(
                    &self.operation,
                    &self.source_ref,
                    target,
                    *status,
                    detail.clone(),
                );
            }
            StreamOutcome::Gap(status, detail) => {
                state.clean = false;
                self.evidence.gap(
                    &self.operation,
                    &self.source_ref,
                    target,
                    *status,
                    detail.clone(),
                );
            }
        }
        state.remaining = state.remaining.saturating_sub(1);
        if state.remaining == 0 && state.clean && self.allow_success {
            self.evidence.success(
                &self.operation,
                &self.source_ref,
                "all",
                "all receipt streams acknowledged".into(),
            );
        }
    }

    fn note_failure(&self, target: &str, status: BackgroundWriteTerminalStatus, detail: String) {
        self.evidence
            .failure(&self.operation, &self.source_ref, target, status, detail);
    }

    pub(super) fn shutdown(&self, target: &str) {
        self.finish(
            target,
            StreamOutcome::Gap(
                BackgroundWriteGapStatus::Shutdown,
                "observer shut down before a terminal receipt".into(),
            ),
        );
    }
}

pub(super) fn run(
    receiver: Arc<Mutex<Receiver<ReceiptJob>>>,
    sender: SyncSender<ReceiptJob>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        let job = receiver
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .recv_timeout(SHUTDOWN_WAKE_CADENCE);
        let job = match job {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if shutdown.load(Ordering::Acquire) {
            job.tracker.shutdown(&job.target);
            return;
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| poll_stream(&job, &shutdown)));
        match outcome {
            Ok(PollOutcome::Terminal(outcome)) => job.tracker.finish(&job.target, outcome),
            Ok(PollOutcome::Pending) => {
                if shutdown.load(Ordering::Acquire) {
                    job.tracker.shutdown(&job.target);
                    return;
                }
                match sender.try_send(job) {
                    Ok(()) => {}
                    Err(TrySendError::Full(job)) => job.tracker.finish(
                        &job.target,
                        StreamOutcome::Gap(
                            BackgroundWriteGapStatus::CapacityFull,
                            "reserved receipt could not re-enter the fair observation queue".into(),
                        ),
                    ),
                    Err(TrySendError::Disconnected(job)) => job.tracker.finish(
                        &job.target,
                        StreamOutcome::Gap(
                            BackgroundWriteGapStatus::ObserverClosed,
                            "receipt observer closed while requeueing a pending stream".into(),
                        ),
                    ),
                }
                continue;
            }
            Err(_) => job.tracker.finish(
                &job.target,
                StreamOutcome::Gap(
                    BackgroundWriteGapStatus::WorkerLost,
                    "receipt worker panicked while observing the stream".into(),
                ),
            ),
        }
        drop(job);
        if shutdown.load(Ordering::Acquire) {
            return;
        }
    }
}

enum StreamOutcome {
    Acked,
    Failure(BackgroundWriteTerminalStatus, String),
    Gap(BackgroundWriteGapStatus, String),
}

enum PollOutcome {
    Terminal(StreamOutcome),
    Pending,
}

fn poll_stream(job: &ReceiptJob, shutdown: &AtomicBool) -> PollOutcome {
    #[cfg(test)]
    if job.target == "panic:test-worker" {
        panic!("scripted receipt observer panic");
    }
    if shutdown.load(Ordering::Acquire) {
        return PollOutcome::Terminal(StreamOutcome::Gap(
            BackgroundWriteGapStatus::Shutdown,
            "observer shut down before a terminal receipt".into(),
        ));
    }
    let remaining = job.deadline.saturating_duration_since(Instant::now());
    match job
        .receiver
        .recv_timeout(remaining.min(SHUTDOWN_WAKE_CADENCE))
    {
        Ok(status) => match classify(status) {
            ReceiptProgress::Intermediate => PollOutcome::Pending,
            ReceiptProgress::Acked => PollOutcome::Terminal(StreamOutcome::Acked),
            ReceiptProgress::Failure(status, detail) => {
                PollOutcome::Terminal(StreamOutcome::Failure(status, detail))
            }
            ReceiptProgress::NonterminalFailure(status, detail) => {
                job.tracker.note_failure(&job.target, status, detail);
                PollOutcome::Pending
            }
            ReceiptProgress::Gap(status, detail) => {
                PollOutcome::Terminal(StreamOutcome::Gap(status, detail))
            }
        },
        Err(FifoRecvTimeoutError::Closed) => PollOutcome::Terminal(StreamOutcome::Gap(
            BackgroundWriteGapStatus::ReceiptDisconnected,
            "receipt stream closed before a terminal receipt".into(),
        )),
        Err(FifoRecvTimeoutError::Timeout) => {
            if Instant::now() >= job.deadline {
                PollOutcome::Terminal(StreamOutcome::Gap(
                    BackgroundWriteGapStatus::ReceiptTimeout,
                    "observation deadline elapsed before a terminal receipt".into(),
                ))
            } else {
                PollOutcome::Pending
            }
        }
        Err(FifoRecvTimeoutError::Lagged) => PollOutcome::Terminal(StreamOutcome::Gap(
            BackgroundWriteGapStatus::ReceiptLagged,
            "receipt stream exceeded its bounded delivery capacity".into(),
        )),
    }
}

enum ReceiptProgress {
    Intermediate,
    Acked,
    Failure(BackgroundWriteTerminalStatus, String),
    NonterminalFailure(BackgroundWriteTerminalStatus, String),
    Gap(BackgroundWriteGapStatus, String),
}

fn classify(status: WriteStatus) -> ReceiptProgress {
    match status {
        WriteStatus::Accepted
        | WriteStatus::Signed(_)
        | WriteStatus::Routed(_)
        | WriteStatus::AwaitingCapability { .. }
        | WriteStatus::AwaitingRelay { .. }
        | WriteStatus::AwaitingAuth { .. }
        | WriteStatus::RetryEligible { .. }
        | WriteStatus::HandoffAmbiguous { .. }
        | WriteStatus::Sent { .. } => ReceiptProgress::Intermediate,
        WriteStatus::Acked(_) => ReceiptProgress::Acked,
        WriteStatus::Cancelled => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::Cancelled,
            "write was cancelled before signature promotion".into(),
        ),
        WriteStatus::Failed(reason) => {
            ReceiptProgress::Failure(BackgroundWriteTerminalStatus::Failed, reason)
        }
        WriteStatus::Rejected(relay, reason) => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::Rejected,
            format!("{relay}: {reason}"),
        ),
        WriteStatus::GaveUp(relay) => {
            ReceiptProgress::Failure(BackgroundWriteTerminalStatus::GaveUp, relay.to_string())
        }
        WriteStatus::PersistenceBlocked(relay) => ReceiptProgress::NonterminalFailure(
            BackgroundWriteTerminalStatus::PersistenceBlocked,
            relay.to_string(),
        ),
        WriteStatus::RoutePersistenceBlocked(relay) => ReceiptProgress::NonterminalFailure(
            BackgroundWriteTerminalStatus::RoutePersistenceBlocked,
            relay.to_string(),
        ),
        WriteStatus::OutcomeUnknown(relay) => {
            ReceiptProgress::Gap(BackgroundWriteGapStatus::OutcomeUnknown, relay.to_string())
        }
        WriteStatus::ReplaceableConflict { expected, actual } => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::ReplaceableConflict,
            format!("expected {expected:?}, actual {actual:?}"),
        ),
    }
}

#[cfg(test)]
#[path = "worker/tests.rs"]
mod tests;
