//! Review round 5: the retry driver cannot spin, on an abandoned lease of an
//! absent event or on a persistently failing store.

use super::*;

fn dispatch_key(state: &Arc<DaemonState>, slug: &str) -> String {
    format!(
        "{SESSION_DISPATCH_CLAIM_PREFIX}{}:{slug}",
        state.backend_pubkey()
    )
}

/// Store a failed, already-due claim for the fixture, as a previous run would.
fn due_retry(state: &Arc<DaemonState>, fixture: &Fixture, key: &str) {
    state.with_store(|store| {
        assert!(store
            .claim_orchestration_target(&fixture.id(), key, "admin", "route", "x", now_secs())
            .unwrap());
        store
            .retry_orchestration_target(&fixture.id(), key, now_secs())
            .unwrap();
        store.rewind_event_claims_for_test(&fixture.id(), DELAY_AFTER[0]);
    });
}

/// Blocker: a run that died mid-flight leaves an expired `processing` lease.
/// If its event is gone from the converged view, the pass must charge it.
#[tokio::test(start_paused = true)]
async fn an_absent_event_with_an_expired_lease_is_charged_not_spun() {
    let state = DaemonState::new_for_test().await;
    let event_id = format!("{:064x}", 9);
    let key = "session-dispatch:backend:gone";
    state.with_store(|store| {
        assert!(store
            .claim_orchestration_target(&event_id, key, "admin", "route", "x", now_secs())
            .unwrap());
        store.rewind_event_claims_for_test(&event_id, 601);
    });
    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    let grace_end = driver.wait(&state).unwrap();
    tokio::time::sleep_until(grace_end).await;

    driver.redispatch_due(&state);

    let claim =
        state.with_store(|store| store.event_claim_for_test(&event_id, "session-dispatch:"));
    let (claim_state, attempts, _) = claim.unwrap();
    assert_eq!((claim_state.as_str(), attempts), ("pending", 1));
    assert!(driver.wait(&state).unwrap() > Instant::now());
}

/// Should-fix: writes failing (full disk) leave the due claim due; passes back
/// off on `min(30s * 2^k, 5min)` instead of re-firing at once.
#[tokio::test(start_paused = true)]
async fn failing_writes_back_the_timer_off_up_to_a_cap() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    due_retry(&state, &fixture, &dispatch_key(&state, "worker"));
    state.with_store(|store| store.set_read_only_for_test(true));

    let mut driver = Driver::new();
    let mut delays = Vec::new();
    for _ in 0..6 {
        driver.redispatch_due(&state);
        let next = driver.wait(&state).unwrap();
        delays.push((next - Instant::now()).as_secs());
        tokio::time::sleep_until(next).await;
    }

    assert_eq!(delays, [30, 60, 120, 240, 300, 300]);
    assert_eq!(
        fixture.state_and_attempts(&state),
        Some(("pending".to_string(), 1))
    );
}

/// Should-fix: progress clears the backoff.
#[tokio::test(start_paused = true)]
async fn a_pass_that_makes_progress_clears_the_backoff() {
    let state = DaemonState::new_for_test().await;
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    due_retry(&state, &fixture, &dispatch_key(&state, "worker"));
    state.with_store(|store| store.set_read_only_for_test(true));
    let mut driver = Driver::new();
    driver.redispatch_due(&state);
    let floor = driver.floor_until().expect("a stalled pass backs off");

    state.with_store(|store| store.set_read_only_for_test(false));
    tokio::time::sleep_until(floor).await;
    driver.redispatch_due(&state);

    assert_eq!(driver.floor_until(), None);
    assert_eq!(fixture.state_and_attempts(&state).unwrap().0, "processing");
}

/// Should-fix: a failing read neither stalls the timer nor spins it.
#[tokio::test(start_paused = true)]
async fn a_failing_scan_backs_off_instead_of_stalling() {
    let state = DaemonState::new_for_test().await;
    state.with_store(|store| store.hide_event_claims_for_test(true));
    let mut driver = Driver::new();

    let next = driver
        .wait(&state)
        .expect("a scan error must not stall retries");
    assert!(next >= Instant::now() + Duration::from_secs(30));
    assert_eq!(
        driver.wait(&state),
        Some(next),
        "one escalation, not one per call"
    );
    state.with_store(|store| store.hide_event_claims_for_test(false));
}

/// Nit: one pass dispatches at most `DUE_BATCH_LIMIT` in-view events.
#[tokio::test]
async fn one_pass_dispatches_at_most_a_batch_of_due_events() {
    let state = DaemonState::new_for_test().await;
    let requester = Keys::generate();
    let relay = Keys::generate();
    let mut events = vec![group_record(&relay, 39_002, "route", &requester)];
    let mut ids = Vec::new();
    // One more than a 64-event batch; literal so the test does not scale
    // with the constant it checks.
    for n in 0..65 {
        let slug = format!("worker-{n}");
        let target = DispatchTarget {
            backend_pubkey: state.backend_pubkey(),
            slug: slug.clone(),
            workspace: "workspace".into(),
            channels: vec!["route".into()],
        };
        let event = build_session_dispatch_event("route", &target, "dispatch")
            .unwrap()
            .tag(Tag::custom("h", ["route"]))
            .finalize(&requester)
            .unwrap();
        let key = dispatch_key(&state, &slug);
        let id = event.id.to_hex();
        state.with_store(|store| {
            assert!(store
                .claim_orchestration_target(&id, &key, "admin", "route", "x", now_secs())
                .unwrap());
            store
                .retry_orchestration_target(&id, &key, now_secs())
                .unwrap();
            store.rewind_event_claims_for_test(&id, DELAY_AFTER[0]);
        });
        ids.push(id);
        events.push(event);
    }
    let view = state.nip29_view();
    view.replace_group_scope(BTreeSet::from(["route".to_string()]), BTreeSet::new());
    view.replace(events);

    Driver::new().redispatch_due(&state);

    let pending = ids
        .iter()
        .filter(|id| {
            state
                .with_store(|store| store.event_claim_for_test(id, SESSION_DISPATCH_CLAIM_PREFIX))
                .unwrap()
                .0
                == "pending"
        })
        .count();
    assert_eq!(pending, 1);
}

/// Nit: a run whose lease was taken over cannot finish the claim.
#[tokio::test]
async fn a_superseded_run_cannot_finish_the_claim() {
    let state = DaemonState::new_for_test().await;
    let key = "session-dispatch:backend:worker";
    let first = state
        .with_store(|store| {
            store.claim_driven_target("ev", key, "a", "route", "x", now_secs() - 601)
        })
        .unwrap()
        .unwrap();
    let second = state
        .with_store(|store| store.claim_driven_target("ev", key, "a", "route", "x", now_secs()))
        .unwrap()
        .unwrap();
    assert_ne!(first, second);

    let row = || state.with_store(|store| store.event_claim_for_test("ev", "session-dispatch:"));
    finish_claim(&state, "ev", key, first, ClaimOutcome::Retry("stale"));
    let (claim_state, attempts, _) = row().unwrap();
    assert_eq!(
        (claim_state.as_str(), attempts),
        ("processing", second),
        "a stale retry cannot send the new owner's run back to pending"
    );
    finish_claim(&state, "ev", key, first, ClaimOutcome::Completed);
    assert_eq!(row().unwrap().0, "processing", "the stale run is ignored");

    finish_claim(&state, "ev", key, second, ClaimOutcome::Completed);
    assert_eq!(row().unwrap().0, "completed");
}
