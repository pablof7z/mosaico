//! Background retry queue for explicit relay-publish retries.
//!
//! Checked publishes report one relay verdict and do not enqueue hidden retries.
//! Callers that know a failed event is safe and useful to retry can push it here;
//! the daemon background task (`spawn_retry_drainer`) drains due entries with
//! exponential backoff and gives up after `MAX_ATTEMPTS`.

use nostr_sdk::prelude::Event;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_ATTEMPTS: u32 = 6;
const BASE_DELAY_MS: u64 = 400;
const MAX_DELAY_MS: u64 = 30_000;

pub struct PendingRetry {
    pub event: Event,
    pub attempt: u32,
    pub retry_after: Instant,
}

#[derive(Default)]
pub struct RetryQueue {
    inner: Mutex<Vec<PendingRetry>>,
}

fn id_short(event: &Event) -> String {
    let hex = event.id.to_hex();
    hex[..8.min(hex.len())].to_string()
}

impl RetryQueue {
    /// Enqueue a failed event for its first retry (backoff starts at `BASE_DELAY_MS`).
    pub fn push_failed(&self, event: Event) {
        let retry_after = Instant::now() + Duration::from_millis(BASE_DELAY_MS);
        eprintln!(
            "[retry] queued event {} kind:{} for retry in {}ms",
            id_short(&event),
            event.kind.as_u16(),
            BASE_DELAY_MS,
        );
        self.inner.lock().unwrap().push(PendingRetry {
            event,
            attempt: 0,
            retry_after,
        });
    }

    /// Take all entries whose `retry_after` has passed, leaving the rest.
    pub fn drain_due(&self) -> Vec<PendingRetry> {
        let now = Instant::now();
        let mut q = self.inner.lock().unwrap();
        let (due, pending): (Vec<_>, Vec<_>) = std::mem::take(&mut *q)
            .into_iter()
            .partition(|r| r.retry_after <= now);
        *q = pending;
        due
    }

    /// Put a still-failing entry back with an incremented attempt counter and
    /// updated retry time, or drop it when `MAX_ATTEMPTS` is exhausted.
    pub fn requeue(&self, mut retry: PendingRetry, reason: &str) {
        retry.attempt += 1;
        if retry.attempt >= MAX_ATTEMPTS {
            eprintln!(
                "[retry] event {} kind:{} exhausted {} attempts, dropping ({})",
                id_short(&retry.event),
                retry.event.kind.as_u16(),
                MAX_ATTEMPTS,
                reason,
            );
            return;
        }
        let delay_ms = (BASE_DELAY_MS * 2u64.pow(retry.attempt)).min(MAX_DELAY_MS);
        retry.retry_after = Instant::now() + Duration::from_millis(delay_ms);
        eprintln!(
            "[retry] event {} kind:{} attempt {}/{} in {}ms ({})",
            id_short(&retry.event),
            retry.event.kind.as_u16(),
            retry.attempt,
            MAX_ATTEMPTS,
            delay_ms,
            reason,
        );
        self.inner.lock().unwrap().push(retry);
    }

    pub fn pending_count(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}
