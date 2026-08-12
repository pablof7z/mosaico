use super::super::*;
use super::reg;

fn count_rows(s: &Store, table: &str) -> i64 {
    s.conn
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn newer_schema_version_fails_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 999u32).unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("newer schema must fail"),
        Err(e) => e,
    };

    assert!(err.to_string().contains("schema version 999"));
    assert!(err.to_string().contains("newer than this binary"));
}

#[test]
fn unstamped_existing_schema_fails_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE legacy_state (id INTEGER)", [])
        .unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("unstamped existing schema must fail"),
        Err(e) => e,
    };

    assert!(err.to_string().contains("no schema version stamp"));
}

#[test]
fn retention_prune_preserves_pending_inbox() {
    let s = Store::open_memory().unwrap();
    s.reserve_hook_session_for_test(&reg("claude-code", "x", "h1"))
        .unwrap();
    s.enqueue_inbox("pending", "pk-agent", "from", "h1", "pending", 1)
        .unwrap();
    s.enqueue_inbox("old-done", "pk-agent", "from", "h1", "old", 1)
        .unwrap();
    s.enqueue_inbox("new-done", "pk-agent", "from", "h1", "new", 1)
        .unwrap();
    s.mark_delivered("old-done", "pk-agent", 1).unwrap();
    s.mark_delivered("new-done", "pk-agent", 10).unwrap();

    let report = s.prune_retained_state_before(5).unwrap();

    assert_eq!(report.delivered_inbox, 1);
    assert_eq!(s.peek_pending_for_pubkey("pk-agent").unwrap().len(), 1);
    assert_eq!(
        s.recently_delivered_for_pubkey("pk-agent", 0)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(count_rows(&s, "inbox"), 2);
}

#[test]
fn retention_keeps_offline_mention_replay_tombstones() {
    let s = Store::open_memory().unwrap();
    assert!(s
        .claim_offline_mention("mention", "agent", "from", "room", "wake up", 1)
        .unwrap());
    s.complete_offline_mention("mention", "agent", 2).unwrap();
    assert!(s
        .claim_management_command("management", "from", "room", "who", 1)
        .unwrap());
    s.complete_management_command("management", 2).unwrap();

    let report = s.prune_retained_state_before(3).unwrap();

    assert_eq!(report.completed_event_claims, 1);
    assert!(!s
        .claim_offline_mention("mention", "agent", "from", "room", "wake up", 4)
        .unwrap());
    assert!(s
        .claim_management_command("management", "from", "room", "who", 4)
        .unwrap());
}

#[test]
fn retention_prunes_finished_native_outcomes_but_keeps_open_attempts() {
    let s = Store::open_memory().unwrap();
    let finished = s
        .start_native_turn_attempt(&NewNativeTurnAttempt {
            pubkey: "pk",
            runtime_generation: 1,
            delivery_kind: NativeTurnDeliveryKind::InboxEvent,
            delivery_event_id: "event",
            native_thread_id: "thread",
            started_at: 1,
        })
        .unwrap();
    s.finish_native_turn_attempt(&FinishNativeTurnAttempt {
        id: finished,
        pubkey: "pk",
        runtime_generation: 1,
        native_turn_id: "turn",
        outcome: NativeTurnOutcome::Completed,
        error_message: "",
        error_details: "",
        finished_at: 2,
    })
    .unwrap();
    s.start_native_turn_attempt(&NewNativeTurnAttempt {
        pubkey: "pk",
        runtime_generation: 1,
        delivery_kind: NativeTurnDeliveryKind::SpawnPrompt,
        delivery_event_id: "",
        native_thread_id: "thread",
        started_at: 1,
    })
    .unwrap();

    let report = s.prune_retained_state_before(3).unwrap();

    assert_eq!(report.native_turn_attempts, 1);
    assert_eq!(count_rows(&s, "native_turn_attempts"), 1);
}
