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
        s.prune_retained_state_before(100).unwrap();
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
fn failed_offline_mention_recovery_can_be_claimed_again() {
    let s = Store::open_memory().unwrap();
    assert!(s
        .claim_offline_mention("event-1", "agent-a", "from", "room", "do it", 100)
        .unwrap());

    s.retry_offline_mention("event-1", "agent-a", 100).unwrap();

    assert!(s
        .list_retryable_offline_mentions(129, 10)
        .unwrap()
        .is_empty());
    assert_eq!(
        s.list_retryable_offline_mentions(130, 10).unwrap(),
        vec![crate::state::event_claims::OfflineMentionClaim {
            event_id: "event-1".into(),
            mentioned_pubkey: "agent-a".into(),
            from_pubkey: "from".into(),
            channel_h: "room".into(),
            body: "do it".into(),
        }]
    );

    assert!(s
        .claim_offline_mention("event-1", "agent-a", "from", "room", "do it", 101)
        .unwrap());
}
