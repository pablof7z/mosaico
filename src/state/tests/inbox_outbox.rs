use super::super::*;
use super::reg;

#[test]
fn inbox_idempotency_and_delivery() {
    let s = Store::open_memory().unwrap();
    s.reserve_session(&reg("claude-code", "x", "h1")).unwrap();
    assert!(s
        .enqueue_inbox("ev1", "pk-agent", "from", "h1", "hi", 100)
        .unwrap());
    // Duplicate is ignored (idempotent).
    assert!(!s
        .enqueue_inbox("ev1", "pk-agent", "from", "h1", "hi", 100)
        .unwrap());
    assert!(s.is_event_handled("ev1", "pk-agent").unwrap());
    assert_eq!(s.peek_pending_for_pubkey("pk-agent").unwrap().len(), 1);
    s.mark_delivered("ev1", "pk-agent", 200).unwrap();
    assert!(s.peek_pending_for_pubkey("pk-agent").unwrap().is_empty());
}

#[test]
fn offline_mention_claim_survives_store_reopen_per_recipient() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    {
        let s = Store::open(&path).unwrap();
        assert!(s
            .claim_offline_mention("event-1", "agent-a", "from", "room", "do it", 100)
            .unwrap());
        s.complete_offline_mention("event-1", "agent-a", 101)
            .unwrap();
        s.prune_retained_state_before(1_000, 1_000).unwrap();
    }

    let reopened = Store::open(&path).unwrap();
    assert!(!reopened
        .claim_offline_mention("event-1", "agent-a", "from", "room", "do it", 200)
        .unwrap());
    assert!(reopened
        .claim_offline_mention("event-1", "agent-b", "from", "room", "do it", 200)
        .unwrap());
}

#[test]
fn outbox_publish_and_retry() {
    let s = Store::open_memory().unwrap();
    let id = s.enqueue_outbox("{\"k\":1}", 100).unwrap();
    assert_eq!(s.peek_outbox(10, u64::MAX).unwrap().len(), 1);
    s.apply_outbox_projection(id, "pending", Some("relay down"), true)
        .unwrap();
    let pending = s.peek_outbox(10, u64::MAX).unwrap();
    assert_eq!(pending[0].retries, 1);
    s.apply_outbox_projection(id, "published", None, false)
        .unwrap();
    assert!(s.peek_outbox(10, u64::MAX).unwrap().is_empty());
}

#[test]
fn outbox_backoff_gates_and_grows() {
    let s = Store::open_memory().unwrap();
    let a = s.enqueue_outbox("{\"k\":1}", 100).unwrap();
    let b = s.enqueue_outbox("{\"k\":2}", 100).unwrap();

    // Fresh rows are due immediately (next_attempt_at defaults to 0).
    assert_eq!(s.peek_outbox(10, 1_000).unwrap().len(), 2);

    // Back row `a` off to t=2000; `b` stays due. A backed-off row must NOT
    // head-of-line-block the still-due `b`.
    s.schedule_outbox_retry(a, 2_000).unwrap();
    let due = s.peek_outbox(10, 1_500).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].local_id, b);

    // Once now passes the backoff, `a` is due again.
    assert_eq!(s.peek_outbox(10, 2_000).unwrap().len(), 2);

    // Delay grows with retries and is capped at 60s (+ up to base/4 jitter).
    let d0 = crate::state::outbox_retry_delay_secs(0, a);
    let d3 = crate::state::outbox_retry_delay_secs(3, a);
    assert!(d0 < d3, "backoff must grow with retries ({d0} !< {d3})");
    assert!(
        crate::state::outbox_retry_delay_secs(50, a) <= 60 + 15,
        "backoff must stay capped"
    );
}
