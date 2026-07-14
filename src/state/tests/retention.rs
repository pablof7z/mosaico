use super::super::*;
use super::reg;

fn count_rows(s: &Store, table: &str) -> i64 {
    s.conn
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[test]
fn incompatible_schema_version_fails_loudly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 999u32).unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("incompatible schema must fail"),
        Err(e) => e,
    };

    assert!(err.to_string().contains("schema version 999"));
    assert!(err.to_string().contains("incompatible"));
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
fn retention_prune_preserves_pending_outbox() {
    let s = Store::open_memory().unwrap();
    let pending = s.enqueue_outbox("{\"pending\":true}", 1).unwrap();
    let old_done = s.enqueue_outbox("{\"old\":true}", 1).unwrap();
    let new_done = s.enqueue_outbox("{\"new\":true}", 10).unwrap();
    s.apply_outbox_projection(old_done, "published", None, false)
        .unwrap();
    s.apply_outbox_projection(new_done, "published", None, false)
        .unwrap();

    let report = s.prune_retained_state_before(0, 5).unwrap();

    assert_eq!(report.published_outbox, 1);
    assert_eq!(s.peek_outbox(10, u64::MAX).unwrap()[0].local_id, pending);
    assert_eq!(count_rows(&s, "outbox"), 2);
}

#[test]
fn retention_prune_preserves_pending_inbox() {
    let s = Store::open_memory().unwrap();
    s.reserve_session(&reg("claude-code", "x", "h1")).unwrap();
    s.enqueue_inbox("pending", "pk-agent", "from", "h1", "pending", 1)
        .unwrap();
    s.enqueue_inbox("old-done", "pk-agent", "from", "h1", "old", 1)
        .unwrap();
    s.enqueue_inbox("new-done", "pk-agent", "from", "h1", "new", 1)
        .unwrap();
    s.mark_delivered("old-done", "pk-agent", 1).unwrap();
    s.mark_delivered("new-done", "pk-agent", 10).unwrap();

    let report = s.prune_retained_state_before(0, 5).unwrap();

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
fn retention_prune_only_safe_rows() {
    let s = Store::open_memory().unwrap();
    let mk = |id: &str, ts: u64| RelayEvent {
        id: id.into(),
        kind: 9,
        pubkey: "pk".into(),
        created_at: ts,
        channel_h: "h1".into(),
        d_tag: String::new(),
        content: String::new(),
        tags_json: "[]".into(),
    };
    assert!(s.insert_event(&mk("old", 1)).unwrap());
    assert!(s.insert_event(&mk("new", 10)).unwrap());
    s.reserve_session(&reg("codex", "alive", "h1")).unwrap();
    s.reserve_session(&reg("codex", "dead", "h1")).unwrap();
    s.mark_dead("dead").unwrap();
    s.put_session_locator("codex", LOCATOR_NATIVE_RESUME, "resume-dead", "dead", 2)
        .unwrap();

    let report = s.prune_retained_state_before(5, 5).unwrap();

    assert_eq!(report.relay_events, 1);
    assert!(s.get_event("old").unwrap().is_none());
    assert!(s.get_event("new").unwrap().is_some());
    assert!(s.get_session("alive").unwrap().is_some());
    assert!(s.get_session("dead").unwrap().is_some());
    assert_eq!(count_rows(&s, "session_locators"), 1);
}
