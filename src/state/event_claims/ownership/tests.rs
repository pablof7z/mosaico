use super::super::retry::MAX_RETRY_ATTEMPTS;
use super::*;

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
fn an_expired_lease_takeover_charges_the_crashed_attempt() {
    let store = Store::open_memory().unwrap();
    let key = "orchestration:backend:0:a::";
    let mut now = 0;
    assert!(store
        .claim_orchestration_target("ev", key, "admin", "child", "a", now)
        .unwrap());
    for takeover in 1..MAX_RETRY_ATTEMPTS {
        now += PROCESSING_LEASE_SECS + 1;
        assert!(store
            .claim_orchestration_target("ev", key, "admin", "child", "a", now)
            .unwrap());
        let attempts: u32 = store
            .conn
            .query_row(
                "SELECT attempts FROM event_claims WHERE claim_key=?1",
                [key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attempts, takeover);
    }
    now += PROCESSING_LEASE_SECS + 1;
    assert!(!store
        .claim_orchestration_target("ev", key, "admin", "child", "a", now)
        .unwrap());
    assert_eq!(state_for(&store, "ev", key), "failed");
}

#[test]
fn a_completed_run_wins_over_a_charge_but_not_over_a_newer_owner() {
    let store = Store::open_memory().unwrap();
    let key = "session-dispatch:backend:worker";
    let first = store
        .claim_driven_target("ev", key, "a", "route", "x", 0)
        .unwrap()
        .unwrap();
    // The lease lapses and the absent event is charged while the run works.
    store
        .charge_absent_driven_claims("ev", PROCESSING_LEASE_SECS + 1)
        .unwrap();
    assert_eq!(state_for(&store, "ev", key), "pending");
    assert!(store
        .complete_unless_reclaimed("ev", key, first, PROCESSING_LEASE_SECS + 2)
        .unwrap());
    assert_eq!(state_for(&store, "ev", key), "completed");

    let other = "session-dispatch:backend:other";
    let stale = store
        .claim_driven_target("ev", other, "a", "route", "x", 0)
        .unwrap()
        .unwrap();
    let newer = store
        .claim_driven_target("ev", other, "a", "route", "x", PROCESSING_LEASE_SECS + 1)
        .unwrap()
        .unwrap();
    assert!(!store
        .complete_unless_reclaimed("ev", other, stale, PROCESSING_LEASE_SECS + 2)
        .unwrap());
    assert!(store.owns_claim("ev", other, newer).unwrap());
}

#[test]
fn a_completed_offline_mention_run_leaves_a_compact_tombstone_in_one_write() {
    let store = Store::open_memory().unwrap();
    let generation = store
        .claim_offline_mention_run("ev", "agent", "sender", "room", "body", 10)
        .unwrap()
        .unwrap();

    assert!(store
        .complete_offline_mention_run("ev", "agent", generation, 11)
        .unwrap());

    let row: (String, String, String, String) = store
        .conn
        .query_row(
            "SELECT state, from_pubkey, channel_h, body FROM event_claims WHERE event_id='ev'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        row,
        (
            "completed".to_string(),
            String::new(),
            String::new(),
            String::new()
        )
    );
}
