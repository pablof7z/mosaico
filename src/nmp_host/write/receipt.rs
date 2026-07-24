//! Interpretation of NMP's durable write receipt stream.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nmp::WriteStatus;
use nostr::EventId;

const WRITE_RECEIPT_TIMEOUT: Duration = Duration::from_secs(12);

pub(super) async fn wait_for_write(
    receivers: Vec<Receiver<WriteStatus>>,
    known_id: Option<EventId>,
    checked: bool,
) -> Result<EventId> {
    tokio::task::spawn_blocking(move || wait_for_write_blocking(receivers, known_id, checked))
        .await
        .context("joining NMP receipt waiter")?
}

pub(super) fn wait_for_write_blocking(
    receivers: Vec<Receiver<WriteStatus>>,
    known_id: Option<EventId>,
    checked: bool,
) -> Result<EventId> {
    let deadline = Instant::now() + WRITE_RECEIPT_TIMEOUT;
    let mut accepted = vec![false; receivers.len()];
    let mut closed = vec![false; receivers.len()];
    let mut event_id = known_id;
    let mut last_status = String::new();
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
                    last_status = format!("failed: {reason}");
                    closed[index] = true;
                }
                Ok(WriteStatus::Rejected(relay, reason)) => {
                    if reason.to_ascii_lowercase().contains("duplicate") {
                        return event_id.context("duplicate NMP write did not report its id");
                    }
                    last_status = format!("rejected by {relay}: {reason}");
                    closed[index] = true;
                }
                Ok(WriteStatus::GaveUp(relay)) => {
                    last_status = format!("gave up delivering to {relay}");
                    closed[index] = true;
                }
                Ok(status) => last_status = format!("{status:?}"),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => closed[index] = true,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
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
            anyhow::bail!("NMP write ended without a relay acknowledgement ({last_status})");
        }
        if Instant::now() >= deadline {
            anyhow::bail!("timed out waiting for NMP write receipt ({last_status})");
        }
    }
}
