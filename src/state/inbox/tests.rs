use super::*;

fn state_for(s: &Store, event_id: &str, target_pubkey: &str) -> String {
    s.conn
        .query_row(
            "SELECT state FROM inbox WHERE event_id=?1 AND target_pubkey=?2",
            params![event_id, target_pubkey],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn inbox_event_prefix_lookup_can_filter_target_pubkey() {
    let s = Store::open_memory().unwrap();
    s.enqueue_inbox("evt-abc", "pk-1", "pk", "room", "one", 10)
        .unwrap();
    s.enqueue_inbox("evt-abc", "pk-2", "pk", "room", "two", 11)
        .unwrap();
    s.enqueue_inbox("evt-other", "pk-1", "pk", "room", "three", 12)
        .unwrap();

    let rows = s.inbox_by_event_prefix("evt-a").unwrap();
    assert_eq!(rows.len(), 2);

    let row = s.inbox_by_event_prefix_and_target("evt-a", "pk-2").unwrap();
    assert_eq!(row.len(), 1);
    assert_eq!(row[0].body, "two");
}

#[test]
fn claim_pending_event_ids_claims_only_the_planned_rows() {
    let s = Store::open_memory().unwrap();
    s.enqueue_inbox("evt-1", "pk-1", "pk", "room", "one", 10)
        .unwrap();
    s.enqueue_inbox("evt-2", "pk-1", "pk", "room", "two", 11)
        .unwrap();

    let rows = s
        .claim_pending_event_ids_for_pubkey(&["evt-2".into()], "pk-1", 20)
        .unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].event_id, "evt-2");
    assert_eq!(state_for(&s, "evt-1", "pk-1"), "pending");
    assert_eq!(state_for(&s, "evt-2", "pk-1"), "delivered");
    assert_eq!(
        s.peek_pending_for_pubkey("pk-1").unwrap()[0].event_id,
        "evt-1"
    );
}

#[test]
fn pending_event_survives_runtime_replacement() {
    let s = Store::open_memory().unwrap();
    upsert_runtime(&s, "pk-agent", 10);
    s.enqueue_inbox("evt", "pk-agent", "sender", "room", "hello", 11)
        .unwrap();
    s.mark_dead("pk-agent").unwrap();
    upsert_runtime(&s, "pk-agent", 12);

    let replacement = s.get_session("pk-agent").unwrap().unwrap();
    assert_eq!(replacement.pubkey, "pk-agent");
    let claimed = s.claim_pending_for_pubkey(&replacement.pubkey, 13).unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].event_id, "evt");
}

#[test]
fn same_event_is_independent_per_pubkey() {
    let s = Store::open_memory().unwrap();
    assert!(s
        .enqueue_inbox("evt", "pk-a", "sender", "room", "hello", 10)
        .unwrap());
    assert!(s
        .enqueue_inbox("evt", "pk-b", "sender", "room", "hello", 10)
        .unwrap());
    assert!(!s
        .enqueue_inbox("evt", "pk-a", "sender", "room", "hello", 10)
        .unwrap());
}

fn upsert_runtime(store: &Store, pubkey: &str, now: u64) {
    store
        .reserve_session(&crate::state::RegisterSession {
            pubkey: pubkey.into(),
            harness: "codex".into(),
            agent_slug: "codex".into(),
            channel_h: "room".into(),
            child_pid: None,
            transcript_path: None,
            now,
        })
        .unwrap();
}
