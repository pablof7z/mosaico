//! Review round 6: a run renews its lease right before each side effect, and
//! a store error on one event does not block the others.

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// SF1: an orchestration claims every target up front and runs them serially.
/// If the driver takes target 1's lease over while target 0 runs, the
/// original run must skip target 1: exactly one run spawns it.
#[tokio::test]
async fn a_target_taken_over_while_earlier_targets_run_is_spawned_once() {
    let state = DaemonState::new_for_test().await;
    let keys = ["orchestration:backend:0:a::", "orchestration:backend:1:b::"];
    let generations: Vec<u32> = keys
        .iter()
        .map(|key| {
            state
                .with_store(|s| s.claim_driven_target("ev", key, "admin", "child", "x", now_secs()))
                .unwrap()
                .unwrap()
        })
        .collect();
    let target_one_spawns = AtomicU32::new(0);

    let ran_zero = run_owned(&state, "ev", keys[0], generations[0], &[], async {
        // Target 0 outlasts target 1's lease; the driver takes target 1 over
        // and its new run spawns it.
        state.with_store(|s| s.rewind_claim_for_test("ev", keys[1], 601));
        let taken_over = state
            .with_store(|s| s.claim_driven_target("ev", keys[1], "admin", "child", "x", now_secs()))
            .unwrap();
        assert!(taken_over.is_some());
        target_one_spawns.fetch_add(1, Ordering::SeqCst);
        ClaimOutcome::Completed
    })
    .await;
    let ran_one = run_owned(&state, "ev", keys[1], generations[1], &[], async {
        target_one_spawns.fetch_add(1, Ordering::SeqCst);
        ClaimOutcome::Completed
    })
    .await;

    assert!(ran_zero);
    assert!(
        !ran_one,
        "the original run must skip a target it no longer owns"
    );
    assert_eq!(target_one_spawns.load(Ordering::SeqCst), 1);
}

/// Round 7, nit 1: each step renews every target the run still holds, so a
/// long serial orchestration keeps its queued targets: they are neither taken
/// over nor charged while earlier targets run.
#[tokio::test]
async fn queued_targets_keep_their_lease_while_earlier_targets_run() {
    let state = DaemonState::new_for_test().await;
    let keys = ["orchestration:backend:0:a::", "orchestration:backend:1:b::"];
    let generations: Vec<u32> = keys
        .iter()
        .map(|key| {
            state
                .with_store(|s| s.claim_driven_target("ev", key, "admin", "child", "x", now_secs()))
                .unwrap()
                .unwrap()
        })
        .collect();
    // Admission and earlier steps already took most of the lease.
    state.with_store(|s| s.rewind_event_claims_for_test("ev", 500));
    let queued = [(keys[1], generations[1])];

    run_owned(&state, "ev", keys[0], generations[0], &queued, async {
        // Target 0 runs another 400s: 900s since the claim, 400s since renewal.
        state.with_store(|s| s.rewind_claim_for_test("ev", keys[1], 400));
        let taken_over = state
            .with_store(|s| s.claim_driven_target("ev", keys[1], "admin", "child", "x", now_secs()))
            .unwrap();
        assert_eq!(taken_over, None, "the queued target's lease was renewed");
        ClaimOutcome::Completed
    })
    .await;

    let ran_one = run_owned(&state, "ev", keys[1], generations[1], &[], async {
        ClaimOutcome::Completed
    })
    .await;
    assert!(
        ran_one,
        "the original run still owns and runs its queued target"
    );
    let row = state
        .with_store(|s| s.event_claim_for_test("ev", "orchestration:backend:1"))
        .unwrap();
    assert_eq!(
        (row.0.as_str(), row.1),
        ("completed", generations[1]),
        "never charged"
    );
}

/// NIT 1: a superseded offline-mention recovery cannot send the new owner's
/// claim back to pending.
#[tokio::test]
async fn a_stale_offline_mention_retry_cannot_reopen_the_claim() {
    let state = DaemonState::new_for_test().await;
    let claim = |now| {
        state
            .with_store(|s| {
                s.claim_offline_mention_run("ev", "agent", "sender", "room", "body", now)
            })
            .unwrap()
    };
    let stale = claim(now_secs() - 601).unwrap();
    let current = claim(now_secs()).unwrap();

    super::super::super::offline_mention::finish_for_test(&state, "ev", "agent", stale, false);

    let row = state
        .with_store(|s| s.event_claim_for_test("ev", "offline-mention:"))
        .unwrap();
    assert_eq!((row.0.as_str(), row.1), ("processing", current));
}

/// NIT 4: a store error on one due event is logged and skipped; the rest of
/// the pass still runs, and the pass backs the timer off.
#[tokio::test(start_paused = true)]
async fn a_store_error_on_one_event_does_not_block_the_pass() {
    let state = DaemonState::new_for_test().await;
    let bad = format!("{:064x}", 1);
    state.with_store(|s| {
        assert!(s
            .claim_orchestration_target(
                &bad,
                "session-dispatch:backend:gone",
                "a",
                "r",
                "x",
                now_secs()
            )
            .unwrap());
        s.retry_orchestration_target(&bad, "session-dispatch:backend:gone", now_secs())
            .unwrap();
        s.rewind_event_claims_for_test(&bad, 3_600);
        s.fail_writes_for_event_for_test(&bad);
    });
    let fixture = Fixture::session_dispatch(&state, "worker");
    fixture.show(&state);
    let key = format!(
        "{SESSION_DISPATCH_CLAIM_PREFIX}{}:worker",
        state.backend_pubkey()
    );
    state.with_store(|s| {
        assert!(s
            .claim_orchestration_target(&fixture.id(), &key, "a", "route", "x", now_secs())
            .unwrap());
        s.retry_orchestration_target(&fixture.id(), &key, now_secs())
            .unwrap();
        s.rewind_event_claims_for_test(&fixture.id(), DELAY_AFTER[0]);
    });
    let mut driver = Driver::new();
    tokio::time::sleep(ABSENCE_GRACE).await;

    driver.redispatch_due(&state);

    assert_eq!(fixture.state_and_attempts(&state).unwrap().0, "processing");
    assert!(
        driver.floor_until().is_some(),
        "the failing event backs the timer off"
    );
}
