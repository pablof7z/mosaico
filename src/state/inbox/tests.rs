use super::*;

fn state_for(s: &Store, event_id: &str, target_key: &str) -> String {
    s.conn
        .query_row(
            "SELECT state FROM inbox WHERE event_id=?1 AND target_session=?2",
            params![event_id, target_key],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn orchestration_target_claim_retries_only_failed_targets() {
    let s = Store::open_memory().unwrap();

    assert!(s
        .claim_orchestration_target("ev", "orchestration:backend:0:a", "admin", "child", "a", 10,)
        .unwrap());
    assert!(s
        .claim_orchestration_target("ev", "orchestration:backend:1:b", "admin", "child", "b", 10,)
        .unwrap());

    s.retry_orchestration_target("ev", "orchestration:backend:0:a")
        .unwrap();
    s.complete_orchestration_target("ev", "orchestration:backend:1:b", 11)
        .unwrap();

    assert!(
        s.claim_orchestration_target("ev", "orchestration:backend:0:a", "admin", "child", "a", 12,)
            .unwrap(),
        "failed target should be retryable"
    );
    assert!(
        !s.claim_orchestration_target(
            "ev",
            "orchestration:backend:1:b",
            "admin",
            "child",
            "b",
            12,
        )
        .unwrap(),
        "completed target should not be reprocessed"
    );
    assert_eq!(
        state_for(&s, "ev", "orchestration:backend:0:a"),
        "processing"
    );
    assert_eq!(
        state_for(&s, "ev", "orchestration:backend:1:b"),
        "delivered"
    );
}
