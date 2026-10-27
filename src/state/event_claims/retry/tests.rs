use super::*;
use std::collections::BTreeSet;

fn state_for(store: &Store, event_id: &str, claim_key: &str) -> String {
    store
        .conn
        .query_row(
            "SELECT state FROM event_claims WHERE event_id=?1 AND claim_key=?2",
            params![event_id, claim_key],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn retry_delay_doubles_from_thirty_seconds_to_an_hour_cap() {
    let delays: Vec<u64> = (1..=9).map(retry_delay_secs).collect();
    assert_eq!(delays, [30, 60, 120, 240, 480, 960, 1920, 3600, 3600]);
    assert_eq!(retry_delay_secs(u32::MAX), RETRY_MAX_DELAY_SECS);
}

#[test]
fn backoff_grows_until_attempts_run_out_then_the_claim_is_terminal() {
    let store = Store::open_memory().unwrap();
    let key = "orchestration:backend:0:a::";
    let mut now = 1_000;
    for attempt in 1..MAX_RETRY_ATTEMPTS {
        assert!(store
            .claim_orchestration_target("ev", key, "admin", "child", "a", now)
            .unwrap());
        let retry = store.retry_orchestration_target("ev", key, now).unwrap();
        let expected_at = now + retry_delay_secs(attempt);
        assert_eq!(
            retry,
            Some(ClaimRetry::Scheduled {
                attempts: attempt,
                retry_at: expected_at,
            })
        );
        assert!(!store
            .claim_orchestration_target("ev", key, "admin", "child", "a", expected_at - 1)
            .unwrap());
        now = expected_at;
    }
    assert!(store
        .claim_orchestration_target("ev", key, "admin", "child", "a", now)
        .unwrap());
    assert_eq!(
        store.retry_orchestration_target("ev", key, now).unwrap(),
        Some(ClaimRetry::Exhausted {
            attempts: MAX_RETRY_ATTEMPTS
        })
    );
    assert_eq!(state_for(&store, "ev", key), "failed");
    assert!(!store
        .claim_orchestration_target("ev", key, "admin", "child", "a", u64::MAX / 2)
        .unwrap());
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        None
    );
}

#[test]
fn a_permanent_failure_is_terminal_at_once() {
    let store = Store::open_memory().unwrap();
    let key = "session-dispatch:backend:ghost";
    assert!(store
        .claim_orchestration_target("ev", key, "admin", "route", "dispatch", 10)
        .unwrap());
    store
        .fail_orchestration_target("ev", key, 10, "test")
        .unwrap();

    assert_eq!(state_for(&store, "ev", key), "failed");
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        None
    );
    assert!(store
        .due_driven_claim_events(u64::MAX / 2, &BTreeSet::new())
        .unwrap()
        .is_empty());
}

#[test]
fn a_restart_inside_the_backoff_still_finds_the_retry_once_due() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let key = "session-dispatch:backend:agent";
    {
        let store = Store::open(&path).unwrap();
        assert!(store
            .claim_orchestration_target("ev", key, "admin", "route", "dispatch", 100)
            .unwrap());
        store.retry_orchestration_target("ev", key, 100).unwrap();
    }

    let reopened = Store::open(&path).unwrap();
    let due_at = 100 + RETRY_BASE_DELAY_SECS;
    assert_eq!(
        reopened.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        Some(due_at)
    );
    assert!(reopened
        .due_driven_claim_events(due_at - 1, &BTreeSet::new())
        .unwrap()
        .is_empty());
    assert_eq!(
        reopened
            .due_driven_claim_events(due_at, &BTreeSet::new())
            .unwrap(),
        ["ev"]
    );
}

#[test]
fn the_timer_wakes_for_driven_claims_and_offline_mentions_only() {
    let store = Store::open_memory().unwrap();
    assert!(store
        .claim_orchestration_target("cmd", "management", "admin", "room", "cmd", 10)
        .unwrap());
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        None
    );

    assert!(store
        .claim_offline_mention("mention", "agent", "sender", "room", "body", 10)
        .unwrap());
    store.retry_offline_mention("mention", "agent", 10).unwrap();
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        Some(10 + RETRY_BASE_DELAY_SECS)
    );
    // Offline mentions recover from their payload, not from the view.
    assert!(store
        .due_driven_claim_events(10 + RETRY_BASE_DELAY_SECS, &BTreeSet::new())
        .unwrap()
        .is_empty());

    for key in [
        format!("{ORCHESTRATION_CLAIM_PREFIX}backend:0:a::"),
        format!("{SESSION_DISPATCH_CLAIM_PREFIX}backend:a"),
    ] {
        assert!(store
            .claim_orchestration_target("driven", &key, "admin", "room", "x", 5)
            .unwrap());
    }
    // An abandoned processing lease is due once the lease expires.
    // A pending (charged) claim completes regardless of generation.
    assert!(store
        .complete_offline_mention_run("mention", "agent", 0, 11)
        .unwrap());
    let lease_due = 5 + PROCESSING_LEASE_SECS + 1;
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        Some(lease_due)
    );
    assert_eq!(
        store
            .due_driven_claim_events(lease_due, &BTreeSet::new())
            .unwrap(),
        ["driven"]
    );
}

#[test]
fn parked_events_do_not_arm_the_timer() {
    let store = Store::open_memory().unwrap();
    for (event, at) in [("early", 10), ("late", 20)] {
        let key = format!("{SESSION_DISPATCH_CLAIM_PREFIX}backend:{event}");
        assert!(store
            .claim_orchestration_target(event, &key, "admin", "room", "x", at)
            .unwrap());
        store.retry_orchestration_target(event, &key, at).unwrap();
    }
    let parked = BTreeSet::from(["early".to_string()]);
    assert_eq!(
        store.next_claim_due_excluding(&parked).unwrap(),
        Some(20 + RETRY_BASE_DELAY_SECS)
    );
    let both = BTreeSet::from(["early".to_string(), "late".to_string()]);
    assert_eq!(store.next_claim_due_excluding(&both).unwrap(), None);
}

#[test]
fn an_event_absent_from_a_converged_view_backs_off_to_terminal() {
    let store = Store::open_memory().unwrap();
    let key = "session-dispatch:backend:agent";
    assert!(store
        .claim_orchestration_target("ev", key, "admin", "route", "x", 100)
        .unwrap());
    store.retry_orchestration_target("ev", key, 100).unwrap();
    let mut now = 100 + RETRY_BASE_DELAY_SECS;

    // Not yet due: nothing is charged.
    store.charge_absent_driven_claims("ev", now - 1).unwrap();
    assert_eq!(
        store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
        Some(now)
    );
    for attempt in 2..MAX_RETRY_ATTEMPTS {
        store.charge_absent_driven_claims("ev", now).unwrap();
        now += retry_delay_secs(attempt);
        assert_eq!(
            store.next_claim_due_excluding(&BTreeSet::new()).unwrap(),
            Some(now)
        );
    }
    store.charge_absent_driven_claims("ev", now).unwrap();
    assert_eq!(state_for(&store, "ev", key), "failed");
}

#[test]
fn parked_events_only_hide_their_view_driven_claims() {
    let store = Store::open_memory().unwrap();
    let mentioned = "agent";
    assert!(store
        .claim_offline_mention("ev", mentioned, "sender", "room", "body", 10)
        .unwrap());
    store.retry_offline_mention("ev", mentioned, 10).unwrap();
    let key = format!("{SESSION_DISPATCH_CLAIM_PREFIX}backend:a");
    assert!(store
        .claim_orchestration_target("ev", &key, "admin", "room", "x", 10)
        .unwrap());
    store.retry_orchestration_target("ev", &key, 10).unwrap();

    let parked = BTreeSet::from(["ev".to_string()]);
    // The offline mention of the same event id still arms the timer.
    assert_eq!(
        store.next_claim_due_excluding(&parked).unwrap(),
        Some(10 + RETRY_BASE_DELAY_SECS)
    );
    assert!(store
        .due_driven_claim_events(10 + RETRY_BASE_DELAY_SECS, &parked)
        .unwrap()
        .is_empty());
}

#[test]
fn offline_mention_retries_are_listed_in_due_order() {
    let store = Store::open_memory().unwrap();
    for event_id in ["older-update", "earlier-due"] {
        assert!(store
            .claim_offline_mention(event_id, "agent", "sender", "room", "body", 1)
            .unwrap());
        store.retry_offline_mention(event_id, "agent", 1).unwrap();
    }
    // Updated first but due last, as after several attempts.
    store.set_claim_schedule_for_test("older-update", 10, 130);
    store.set_claim_schedule_for_test("earlier-due", 20, 50);

    let first = store.list_retryable_offline_mentions(200, 1).unwrap();
    assert_eq!(first[0].event_id, "earlier-due");
    let earliest = store.earliest_due_claim(&BTreeSet::new()).unwrap().unwrap();
    assert_eq!(earliest.event_id, "earlier-due");
}
