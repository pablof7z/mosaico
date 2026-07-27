use super::*;

#[test]
fn channel_invalidation_clears_all_members_for_that_channel_only() {
    let readiness = ChannelReadiness::default();
    readiness.mark_ready("chan-a", "alice");
    readiness.mark_ready("chan-a", "bob");
    readiness.mark_ready("chan-b", "alice");

    readiness.invalidate_channel("chan-a");

    assert!(!readiness.check("chan-a", "alice").0);
    assert!(!readiness.check("chan-a", "bob").0);
    assert!(readiness.check("chan-b", "alice").0);
}

#[test]
fn relay_parent_state_precedes_pending_host_context() {
    assert_eq!(
        effective_parent_hint(Some("relay-parent".into()), Some("host-parent"), "room"),
        Some("relay-parent".into())
    );
    assert_eq!(
        effective_parent_hint(Some(String::new()), Some("host-parent"), "room"),
        None,
        "an observed relay root must suppress the fallback"
    );
    assert_eq!(
        effective_parent_hint(None, Some("host-parent"), "room"),
        Some("host-parent".into())
    );
}

#[tokio::test]
async fn timeout_wrapping_maps_stalled_readiness_to_degraded_bail() {
    use std::time::Duration;

    async fn ensure_ready_bounded(
        timeout: Duration,
        ready: impl std::future::Future<Output = ChannelGate>,
    ) -> anyhow::Result<()> {
        let gate = match tokio::time::timeout(timeout, ready).await {
            Ok(gate) => gate,
            Err(_) => {
                ChannelGate::Degraded(ChannelReadinessError::reason("channel readiness timed out"))
            }
        };
        gate.require_ready("channel is not ready for remote invite")
    }

    let stalled = std::future::pending::<ChannelGate>();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        ensure_ready_bounded(Duration::from_millis(10), stalled),
    )
    .await
    .expect("the bounded wrapper must not hang past its own timeout");
    let error = result.expect_err("a stalled readiness probe must surface an error");
    assert!(
        error.to_string().contains("not ready for remote invite"),
        "unexpected error: {error}"
    );

    let ready = ensure_ready_bounded(Duration::from_millis(10), async { ChannelGate::Ready }).await;
    assert!(ready.is_ok());
}
