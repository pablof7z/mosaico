use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nmp::{FifoReceiver, FifoRecvTimeoutError, WriteFact};

mod facts;

use facts::LaneFacts;

use super::admission::ReceiptSlot;
use super::evidence::{BackgroundWriteGapStatus, BackgroundWriteTerminalStatus, Evidence};

const SHUTDOWN_WAKE_CADENCE: Duration = Duration::from_millis(100);

pub(super) struct ReceiptJob {
    pub(super) receiver: FifoReceiver<WriteFact>,
    pub(super) target: String,
    pub(super) deadline: Instant,
    pub(super) tracker: Arc<Tracker>,
    pub(super) _slot: ReceiptSlot,
    /// What this write's relay lanes have said so far. The whole write ends on
    /// exactly one [`nmp::WriteOutcome`]; these are the per-relay facts that
    /// give that outcome its meaning.
    pub(super) lanes: LaneFacts,
}

struct TrackerState {
    remaining: usize,
    clean: bool,
    /// A superseded write reached no relay, so calling it acknowledged would
    /// be a lie — but it is not a failure either. It is the ordinary outcome
    /// of renewing a replaceable coordinate faster than the older write left
    /// the queue, and reporting it as a fault would fill `mosaico doctor` with
    /// warnings about the steady state (mosaico#745 asked for this call to be
    /// made deliberately; this is it).
    success_eligible: bool,
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
                success_eligible: true,
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
            StreamOutcome::Superseded => state.success_eligible = false,
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
        if state.remaining == 0 && state.clean && state.success_eligible && self.allow_success {
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
        let mut job = match job {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if shutdown.load(Ordering::Acquire) {
            job.tracker.shutdown(&job.target);
            return;
        }
        let outcome = catch_unwind(AssertUnwindSafe(|| poll_stream(&mut job, &shutdown)));
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

pub(super) enum StreamOutcome {
    Acked,
    /// Terminal, clean, and NOT a success: a newer write won the same
    /// replaceable coordinate before this one reached the wire.
    Superseded,
    Failure(BackgroundWriteTerminalStatus, String),
    Gap(BackgroundWriteGapStatus, String),
}

enum PollOutcome {
    Terminal(StreamOutcome),
    Pending,
}

fn poll_stream(job: &mut ReceiptJob, shutdown: &AtomicBool) -> PollOutcome {
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
        Ok(fact) => observe(job, fact),
        Err(FifoRecvTimeoutError::Closed) => PollOutcome::Terminal(StreamOutcome::Gap(
            BackgroundWriteGapStatus::ReceiptDisconnected,
            "receipt stream closed before its write outcome".into(),
        )),
        Err(FifoRecvTimeoutError::Timeout) => {
            if Instant::now() >= job.deadline {
                // NMP never abandons a write on a clock, and neither does
                // this: the obligation stays in NMP's publish queue, readable
                // and removable there. What ends here is one process-local
                // OBSERVATION, which is why it is filed as a gap and never as
                // a write failure.
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

/// Fold one NMP fact into this stream's picture.
///
/// There is no Mosaico classifier here any more. NMP's vocabulary separates
/// whole-write facts from per-relay facts and ends every stream with exactly
/// one [`nmp::WriteOutcome`], so "is this over?" is read off the fact rather
/// than re-derived from a table this repo used to maintain in four places.
fn observe(job: &mut ReceiptJob, fact: WriteFact) -> PollOutcome {
    match fact {
        WriteFact::Signing(signing) => match facts::signer_refusal(signing) {
            Some(reason) => PollOutcome::Terminal(StreamOutcome::Failure(
                BackgroundWriteTerminalStatus::SignerRefused,
                reason,
            )),
            None => PollOutcome::Pending,
        },
        // The intended destination set and whether resolution is closed. Data,
        // not an outcome — and a write still learning where it goes parks
        // indefinitely by design.
        WriteFact::Destinations { .. } => PollOutcome::Pending,
        WriteFact::Relay { relay, state } => {
            if let Some((status, detail)) = job.lanes.observe_relay(&relay, state) {
                job.tracker.note_failure(&job.target, status, detail);
            }
            PollOutcome::Pending
        }
        WriteFact::Outcome(outcome) => PollOutcome::Terminal(job.lanes.settle(outcome)),
    }
}

#[cfg(test)]
#[path = "worker/tests.rs"]
mod tests;
