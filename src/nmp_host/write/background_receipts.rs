use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use anyhow::{Context, Result};
use nmp::{FifoReceiver, WriteStatus};
use nostr::EventId;

mod admission;
mod evidence;
mod worker;

use admission::{Admission, BackgroundReceiptPermit};
use evidence::Evidence;
pub(crate) use evidence::{
    BackgroundWriteGapStatus, BackgroundWriteSnapshot, BackgroundWriteTerminalStatus,
};
use worker::{run as run_worker, ReceiptJob, Tracker};

const BACKGROUND_WRITE_CAPACITY: usize = 128;
const BACKGROUND_WORKERS: usize = 4;
const BACKGROUND_RECEIPT_TIMEOUT: Duration = Duration::from_secs(12);

pub(crate) struct BackgroundReceiptObserver {
    admission: Arc<Admission>,
    evidence: Arc<Evidence>,
    shutdown: Arc<AtomicBool>,
    sender: Mutex<Option<SyncSender<ReceiptJob>>>,
    receiver: Arc<Mutex<Receiver<ReceiptJob>>>,
    workers: Mutex<Vec<JoinHandle<()>>>,
    configured_workers: usize,
    timeout: Duration,
}

impl BackgroundReceiptObserver {
    pub(crate) fn start() -> Result<Self> {
        Self::start_with(
            BACKGROUND_WRITE_CAPACITY,
            BACKGROUND_WORKERS,
            BACKGROUND_RECEIPT_TIMEOUT,
        )
    }

    pub(super) fn start_with(
        capacity: usize,
        worker_count: usize,
        timeout: Duration,
    ) -> Result<Self> {
        let admission = Arc::new(Admission::new(capacity));
        let evidence = Arc::new(Evidence::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel(capacity);
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
            let shutdown = Arc::clone(&shutdown);
            workers.push(
                std::thread::Builder::new()
                    .name(format!("nmp-background-receipt-{index}"))
                    .spawn({
                        let sender = sender.clone();
                        move || run_worker(receiver, sender, shutdown)
                    })
                    .context("starting bounded NMP background receipt worker")?,
            );
        }
        Ok(Self {
            admission,
            evidence,
            shutdown,
            sender: Mutex::new(Some(sender)),
            receiver,
            workers: Mutex::new(workers),
            configured_workers: worker_count,
            timeout,
        })
    }

    /// Reserve every stream slot before the first `Engine::publish`. Full or
    /// closed admission is a correlated observer gap, never a write failure.
    pub(super) fn reserve(
        &self,
        operation: &str,
        event_id: EventId,
        streams: usize,
    ) -> Result<BackgroundReceiptPermit> {
        let source_ref = event_id.to_hex();
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let available = self.admission.capacity.saturating_sub(state.pending);
        let gap = if state.closed {
            Some((
                BackgroundWriteGapStatus::ObserverClosed,
                "background receipt observer is closed",
            ))
        } else if streams > available {
            Some((
                BackgroundWriteGapStatus::CapacityFull,
                "background receipt observer has insufficient stream capacity",
            ))
        } else {
            None
        };
        if let Some((status, detail)) = gap {
            drop(state);
            self.evidence.gap(
                operation,
                &source_ref,
                "admission",
                status,
                detail.to_string(),
            );
            anyhow::bail!("{detail}");
        }
        state.pending += streams;
        Ok(BackgroundReceiptPermit {
            admission: Arc::clone(&self.admission),
            unassigned: streams,
            deadline: Instant::now() + self.timeout,
        })
    }

    pub(super) fn observe(
        &self,
        mut permit: BackgroundReceiptPermit,
        operation: &str,
        event_id: EventId,
        receivers: Vec<(String, FifoReceiver<WriteStatus>)>,
        allow_success: bool,
    ) -> Result<()> {
        let source_ref = event_id.to_hex();
        let tracker = Arc::new(Tracker::new(
            operation,
            &source_ref,
            receivers.len(),
            allow_success,
            Arc::clone(&self.evidence),
        ));
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .context("background NMP receipt observer is closed")?;
        for (target, receiver) in receivers {
            let job = ReceiptJob {
                receiver,
                target,
                deadline: permit.deadline,
                tracker: Arc::clone(&tracker),
                _slot: permit.take_slot(),
            };
            match sender.try_send(job) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    self.evidence.gap(
                        operation,
                        &source_ref,
                        "queue",
                        BackgroundWriteGapStatus::CapacityFull,
                        "reserved background receipt queue unexpectedly full".into(),
                    );
                    anyhow::bail!("reserved background receipt queue unexpectedly full");
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.evidence.gap(
                        operation,
                        &source_ref,
                        "queue",
                        BackgroundWriteGapStatus::ObserverClosed,
                        "background receipt observer stopped".into(),
                    );
                    anyhow::bail!("background receipt observer stopped");
                }
            }
        }
        Ok(())
    }

    pub(super) fn submission_failed(
        &self,
        operation: &str,
        event_id: EventId,
        error: &anyhow::Error,
    ) {
        self.evidence.failure(
            operation,
            &event_id.to_hex(),
            "submission",
            BackgroundWriteTerminalStatus::SubmissionFailed,
            format!("{error:#}"),
        );
    }

    pub(super) fn snapshot(&self) -> BackgroundWriteSnapshot {
        let state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let pending = state.pending;
        let admission_open = !state.closed;
        drop(state);
        let workers = self
            .workers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .filter(|worker| !worker.is_finished())
            .count();
        self.evidence.snapshot(
            pending,
            self.admission.capacity,
            admission_open,
            workers,
            self.configured_workers,
        )
    }

    pub(crate) fn begin_shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.closed = true;
        self.admission.changed.notify_all();
    }

    pub(crate) fn shutdown(&self) {
        self.begin_shutdown();
        self.sender
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        for worker in self
            .workers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drain(..)
        {
            if worker.join().is_err() {
                tracing::error!("background NMP receipt worker panicked during shutdown");
            }
        }
        while let Ok(job) = self
            .receiver
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .try_recv()
        {
            job.tracker.shutdown(&job.target);
        }
    }

    #[cfg(test)]
    pub(crate) fn wait_idle(&self) {
        let state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        drop(
            self.admission
                .changed
                .wait_while(state, |state| state.pending > 0)
                .unwrap_or_else(|poison| poison.into_inner()),
        );
    }
}

impl Drop for BackgroundReceiptObserver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
#[path = "background_receipts/tests.rs"]
mod tests;
