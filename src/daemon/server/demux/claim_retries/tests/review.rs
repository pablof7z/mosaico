//! The round-3 review's repros: the driver never charges an attempt for a
//! run that did not happen.

use super::*;

/// Review round 3, repro 1, plus: every attempt up to the 3rd demonstrably ran.
#[tokio::test]
async fn review_second_due_retry_after_a_real_rerun_is_not_burned() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    super::super::apply_current(&state).await;
    fixture.until(&state, ("pending", 1), PATIENCE).await;

    let mut driver = Driver::new();
    for (attempt, delay) in (2..=3).zip(DELAY_AFTER) {
        fixture.rewind(&state, delay);
        state.with_store(|store| store.scrub_event_claim_requester_for_test(&fixture.id()));

        driver.redispatch_due(&state);

        let (claim_state, attempts, requester) = fixture.claim(&state).unwrap();
        assert_eq!(
            (claim_state.as_str(), attempts, requester),
            ("processing", attempt - 1, fixture.requester()),
            "the due pass must claim (and so run) the handler"
        );
        fixture.until(&state, ("pending", attempt), PATIENCE).await;

        // A pass inside the new backoff neither runs nor charges anything.
        driver.redispatch_due(&state);
        assert_eq!(
            fixture.state_and_attempts(&state),
            Some(("pending".to_string(), attempt))
        );
    }
}

/// Review round 3, repro 2: the claim happens before any relay work, so a slow
/// pre-claim phase cannot be overtaken by another pass.
#[tokio::test(start_paused = true)]
async fn dispatch_claims_before_any_relay_work() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::orchestration(&state, "worker", false);
    fixture.show(&state);
    super::super::apply_current(&state).await;
    fixture.until(&state, ("pending", 1), PATIENCE).await;
    fixture.rewind(&state, DELAY_AFTER[0]);

    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("processing".to_string(), 1)),
        "dispatch claims before the handler's relay work"
    );
    // Only the crash lease of the in-flight claim arms the timer, minutes out.
    let lease = driver.wait(&state).expect("processing lease is tracked");
    assert!(lease > Instant::now() + Duration::from_secs(60));
    driver.redispatch_due(&state);
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("processing".to_string(), 1))
    );

    fixture.until(&state, ("pending", 2), PATIENCE).await;
}

/// Review round 3, repro 3: before the view holds the event, a due retry is parked,
/// not charged, and does not keep the timer hot.
#[tokio::test]
async fn an_absent_event_is_not_charged_before_the_view_converges() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    super::super::apply_current(&state).await;
    fixture.until(&state, ("pending", 1), PATIENCE).await;
    fixture.rewind(&state, DELAY_AFTER[0]);
    state.nip29_view().replace(Vec::new());

    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("pending".to_string(), 1))
    );
    // Only the end of the convergence grace window re-arms the timer.
    let next = driver.wait(&state).unwrap();
    assert!(
        next > Instant::now() + Duration::from_secs(60),
        "a parked event must not re-arm the timer"
    );

    fixture.show(&state);
    driver.view_changed();
    assert!(driver.wait(&state).unwrap() <= Instant::now());
    driver.redispatch_due(&state);
    fixture.until(&state, ("pending", 2), PATIENCE).await;
}
