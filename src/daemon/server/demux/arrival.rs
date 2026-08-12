//! Durable product-local ordering before an NMP Row enters product dispatch.

use std::time::Duration;

use anyhow::Result;

use super::*;

const INITIAL_RETRY: Duration = Duration::from_millis(25);
const MAX_RETRY: Duration = Duration::from_secs(1);

/// Record the host-local arrival fence before any fallible or observable
/// product handling. A storage outage backpressures this one serialized demux
/// instead of silently dropping a Row that NMP already delivered.
pub(super) async fn record_before_dispatch(state: &Arc<DaemonState>, event_id: &str) -> u64 {
    retry(event_id, || {
        state.with_store(|store| store.record_nmp_arrival(event_id))
    })
    .await
}

async fn retry(event_id: &str, mut record: impl FnMut() -> Result<u64>) -> u64 {
    let mut failures = 0_u64;
    let mut delay = INITIAL_RETRY;
    loop {
        match record() {
            Ok(sequence) => {
                if failures > 0 {
                    tracing::info!(%event_id, failures, sequence, "local NMP arrival cursor recovered");
                }
                return sequence;
            }
            Err(error) => {
                failures = failures.saturating_add(1);
                if failures.is_power_of_two() {
                    tracing::warn!(
                        %event_id,
                        failures,
                        retry_ms = delay.as_millis(),
                        %error,
                        "local NMP arrival cursor unavailable; product dispatch is paused"
                    );
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay = delay.saturating_mul(2).min(MAX_RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transient_failure_retries_before_product_dispatch_can_continue() {
        let mut attempts = 0_u8;
        let sequence = retry("event", || {
            attempts += 1;
            if attempts == 1 {
                anyhow::bail!("injected storage outage");
            }
            Ok(7)
        })
        .await;

        assert_eq!(sequence, 7);
        assert_eq!(attempts, 2);
    }
}
