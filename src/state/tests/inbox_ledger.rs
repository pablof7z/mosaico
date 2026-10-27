use super::super::*;
use super::reg;

#[test]
fn inbox_idempotency_and_delivery() {
    let s = Store::open_memory().unwrap();
    s.reserve_hook_session_for_test(&reg("claude-code", "x", "h1"))
        .unwrap();
    assert!(s
        .enqueue_inbox("ev1", "pk-agent", "from", "h1", "hi", 100)
        .unwrap());
    assert!(!s
        .enqueue_inbox("ev1", "pk-agent", "from", "h1", "hi", 100)
        .unwrap());
    assert!(s.is_event_handled("ev1", "pk-agent").unwrap());
    assert_eq!(s.peek_pending_for_pubkey("pk-agent").unwrap().len(), 1);
    s.mark_delivered("ev1", "pk-agent", 200).unwrap();
    assert!(s.peek_pending_for_pubkey("pk-agent").unwrap().is_empty());
}

#[test]
fn park_direct_mention_is_idempotent_across_store_reopen() {
    // The #837 "no per-boot re-side-effects" requirement: a mention parked in
    // a prior daemon run is not re-parked on restart — no duplicate doorbell,
    // no duplicate inbox row. The durable `inbox` table's
    // `(event_id, target_pubkey)` uniqueness is the cross-restart idempotency
    // that retires `first_sight`'s per-process LRU: restart re-routes the
    // same event, the durable claim rejects it.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    {
        let s = Store::open(&path).unwrap();
        assert!(s
            .park_direct_mention("ev1", "pk-agent", "from", "h1", "hi", 100)
            .unwrap());
    }

    let reopened = Store::open(&path).unwrap();
    assert!(
        !reopened
            .park_direct_mention("ev1", "pk-agent", "from", "h1", "hi", 200)
            .unwrap(),
        "restart must not re-park an already-handled mention"
    );
    assert!(reopened.is_event_handled("ev1", "pk-agent").unwrap());
    assert_eq!(
        reopened.peek_pending_for_pubkey("pk-agent").unwrap().len(),
        1
    );
}
