//! Interpretation of NMP's durable write receipt stream.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nmp::{FifoReceiver, FifoRecvTimeoutError, WriteStatus};
use nostr::EventId;

const WRITE_RECEIPT_TIMEOUT: Duration = Duration::from_secs(12);

pub(super) async fn wait_for_write(
    receivers: Vec<FifoReceiver<WriteStatus>>,
    known_id: Option<EventId>,
    checked: bool,
) -> Result<EventId> {
    tokio::task::spawn_blocking(move || wait_for_write_blocking(receivers, known_id, checked))
        .await
        .context("joining NMP receipt waiter")?
}

pub(super) fn wait_for_write_blocking(
    receivers: Vec<FifoReceiver<WriteStatus>>,
    known_id: Option<EventId>,
    checked: bool,
) -> Result<EventId> {
    let deadline = Instant::now() + WRITE_RECEIPT_TIMEOUT;
    let mut accepted = vec![false; receivers.len()];
    let mut closed = vec![false; receivers.len()];
    let mut event_id = known_id;
    let mut last_failure = None;
    loop {
        for (index, receiver) in receivers.iter().enumerate() {
            if closed[index] {
                continue;
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(WriteStatus::Accepted) => accepted[index] = true,
                Ok(WriteStatus::Signed(id)) => event_id = Some(id),
                Ok(WriteStatus::Acked(_)) => {
                    return event_id.context("NMP acknowledged a write before reporting its id");
                }
                Ok(WriteStatus::Failed(reason)) => {
                    anyhow::bail!("NMP write failed: {reason}");
                }
                Ok(WriteStatus::Cancelled) => {
                    anyhow::bail!("NMP write was cancelled");
                }
                Ok(WriteStatus::Rejected(relay, reason)) => {
                    last_failure = Some(format!("rejected by {relay}: {reason}"));
                    closed[index] = true;
                }
                Ok(WriteStatus::GaveUp(relay)) => {
                    last_failure = Some(format!("gave up delivering to {relay}"));
                    closed[index] = true;
                }
                Ok(WriteStatus::PersistenceBlocked(relay)) => {
                    last_failure = Some(format!("persistence blocked for {relay}"));
                }
                Ok(WriteStatus::RoutePersistenceBlocked(relay)) => {
                    last_failure = Some(format!("route persistence blocked for {relay}"));
                }
                Ok(WriteStatus::OutcomeUnknown(relay)) => {
                    last_failure = Some(format!("delivery outcome unknown for {relay}"));
                    closed[index] = true;
                }
                Ok(WriteStatus::ReplaceableConflict { expected, actual }) => {
                    anyhow::bail!(
                        "NMP replaceable write conflicted (expected {expected:?}, actual {actual:?})"
                    );
                }
                Ok(_) => {}
                Err(FifoRecvTimeoutError::Closed) => closed[index] = true,
                Err(FifoRecvTimeoutError::Timeout) => {}
                Err(FifoRecvTimeoutError::Lagged) => {
                    anyhow::bail!("NMP write receipt stream lagged");
                }
            }
        }
        let settled = accepted
            .iter()
            .zip(&closed)
            .all(|(accepted, closed)| *accepted || *closed);
        if !checked && settled && accepted.iter().any(|accepted| *accepted) {
            if let Some(id) = event_id {
                return Ok(id);
            }
        }
        if closed.iter().all(|closed| *closed) {
            let detail = last_failure.as_deref().unwrap_or("receipt streams closed");
            anyhow::bail!("NMP write ended without a relay acknowledgement ({detail})");
        }
        if Instant::now() >= deadline {
            let detail = last_failure
                .as_deref()
                .unwrap_or("no terminal failure observed");
            anyhow::bail!("timed out waiting for NMP write receipt ({detail})");
        }
    }
}
