//! Review round 4: admission before the roster converges, parked starvation,
//! foreign claim keys, and re-arming through the demux loop.

use super::*;

/// Store a failed, already-due claim for `event_id`, as a previous run would.
fn due_retry(state: &Arc<DaemonState>, event_id: &str, key: &str) {
    state.with_store(|store| {
        assert!(store
            .claim_orchestration_target(event_id, key, "admin", "route", "x", now_secs())
            .unwrap());
        store
            .retry_orchestration_target(event_id, key, now_secs())
            .unwrap();
        store.rewind_event_claims_for_test(event_id, DELAY_AFTER[0]);
    });
}

/// Repro 1 (session dispatch): a roster missing its member list is not a
/// rejection. Once the 39002 arrives, the next pass gets past admission (here
/// to the unknown-slug spawn failure).
#[tokio::test]
async fn a_dispatch_admitted_only_after_its_roster_converges_still_runs() {
    let state = DaemonState::new_for_test().await;
    let _workspace = bind_workspace(&state, &["workspace"]);
    let fixture = Fixture::session_dispatch(&state, "no-such-agent-anywhere");
    fixture.without_record(39_002).show(&state);
    super::super::super::apply_current(&state).await;
    fixture.until(&state, ("pending", 1), PATIENCE).await;

    fixture.show(&state);
    fixture.rewind(&state, DELAY_AFTER[0]);
    Driver::new().redispatch_due(&state);

    fixture.until(&state, ("failed", 1), PATIENCE).await;
}

/// Repro 1 (orchestration): the same for an admin list that arrives late.
#[tokio::test]
async fn an_orchestration_admitted_only_after_its_admins_converge_still_runs() {
    let state = DaemonState::new_for_test().await;
    let _workspace = bind_workspace(&state, &["parent", "child"]);
    let fixture = Fixture::orchestration(&state, "no-such-agent-anywhere", true);
    fixture.without_record(39_001).show(&state);
    super::super::super::apply_current(&state).await;
    fixture.until(&state, ("pending", 1), PATIENCE).await;

    fixture.show(&state);
    fixture.rewind(&state, DELAY_AFTER[0]);
    Driver::new().redispatch_due(&state);

    fixture.until(&state, ("failed", 1), PATIENCE).await;
}

/// Repro 2: many parked events due earlier neither starve an in-view due
/// event nor keep the timer firing.
#[tokio::test]
async fn parked_events_neither_starve_due_work_nor_spin_the_timer() {
    let state = DaemonState::new_for_test().await;
    for absent in 0..64 {
        let event_id = format!("{absent:064x}");
        due_retry(&state, &event_id, "session-dispatch:backend:gone");
        state.with_store(|store| store.rewind_event_claims_for_test(&event_id, 10));
    }
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    let key = format!(
        "{SESSION_DISPATCH_CLAIM_PREFIX}{}:worker",
        state.backend_pubkey()
    );
    due_retry(&state, &fixture.id(), &key);

    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    assert_eq!(
        fixture.state_and_attempts(&state).unwrap().0,
        "processing",
        "the in-view due event is dispatched in the first pass"
    );
    for _ in 0..5 {
        let next = driver.wait(&state).unwrap();
        assert!(
            next > Instant::now(),
            "parked events must not re-fire the timer"
        );
        driver.redispatch_due(&state);
    }
}

/// Once the view has had the grace window to converge, a due event still
/// absent is charged, so parked rows cannot accumulate forever.
#[tokio::test(start_paused = true)]
async fn an_absent_event_is_charged_once_the_view_has_converged() {
    let state = DaemonState::new_for_test().await;
    let key = "session-dispatch:backend:gone";
    let event_id = format!("{:064x}", 7);
    due_retry(&state, &event_id, key);
    let attempts =
        || state.with_store(|store| store.event_claim_for_test(&event_id, "session-dispatch:"));

    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    assert_eq!(
        attempts().unwrap().1,
        1,
        "not charged inside the grace window"
    );
    let grace_end = driver.wait(&state).unwrap();
    assert!(grace_end > Instant::now());

    tokio::time::sleep_until(grace_end).await;
    driver.redispatch_due(&state);

    let (claim_state, charged, _) = attempts().unwrap();
    assert_eq!((claim_state.as_str(), charged), ("pending", 2));
    assert!(driver.wait(&state).unwrap() > Instant::now());
}

/// A stored claim key this backend's dispatch never produces (here, one
/// naming a previous backend identity) is terminal; the real one runs.
#[tokio::test]
async fn a_claim_key_this_backend_never_produces_is_terminal() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    let foreign = format!("{SESSION_DISPATCH_CLAIM_PREFIX}previous-backend:worker");
    due_retry(&state, &fixture.id(), &foreign);

    let mut driver = Driver::new();
    driver.redispatch_due(&state);

    let foreign_state = state.with_store(|store| {
        store.event_claim_for_test(
            &fixture.id(),
            &format!("{SESSION_DISPATCH_CLAIM_PREFIX}previous"),
        )
    });
    assert_eq!(foreign_state.unwrap().0, "failed");
    assert_eq!(fixture.state_and_attempts(&state).unwrap().0, "processing");
    let next = driver.wait(&state).unwrap();
    assert!(next > Instant::now(), "nothing is left due");
}

/// Through the demux loop: a parked event that reaches the view is
/// un-parked, so its next retry fires on time rather than at the grace end.
#[tokio::test]
async fn a_view_change_rearms_a_parked_event_through_the_loop() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    let key = format!(
        "{SESSION_DISPATCH_CLAIM_PREFIX}{}:worker",
        state.backend_pubkey()
    );
    due_retry(&state, &fixture.id(), &key);
    super::super::super::spawn_demux(state.clone());
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("pending".to_string(), 1)),
        "absent at startup: parked, not charged"
    );

    fixture.show(&state);
    fixture.until(&state, ("pending", 2), PATIENCE).await;

    tokio::time::pause();
    fixture.rewind(&state, DELAY_AFTER[1]);
    tokio::time::advance(Duration::from_secs(DELAY_AFTER[1])).await;
    fixture.until(&state, ("pending", 3), PATIENCE).await;
}
