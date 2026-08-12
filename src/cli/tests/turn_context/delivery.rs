use super::*;

/// Ambient channel chat is delta-gated off the NMP delivery: a row newer than
/// the cursor surfaces, an older one does not re-emit on the next tool call.
#[test]
fn turn_check_chat_shown_once_not_per_tool_call() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    // A kind:9 chat event in `proj`, created at 120 (after the cursor 50).
    install_relay_delivery(
        &store,
        [],
        [crate::state::RelayEvent {
            id: "chat-new".to_string(),
            kind: 9,
            pubkey: "pk-chat".to_string(),
            created_at: 120,
            channel_h: "proj".to_string(),
            d_tag: String::new(),
            content: "ambient chatter".to_string(),
            tags_json: "[]".to_string(),
        }],
    );
    let m = Mutex::new(store);

    let text = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200)
        .expect("fresh chat past the cursor must surface");
    assert!(
        text.contains("<chatter>"),
        "chat should render inside the unified fabric update; got: {text:?}"
    );
    assert!(
        text.contains("ambient chatter"),
        "chat activity section expected; got: {text:?}"
    );
    // Cursor advanced past the row (since=150 > 120): no re-emit.
    let text2 = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(150), 200);
    assert!(
        text2.is_none(),
        "already-shown chat must not repeat once the cursor passes it; got: {text2:?}"
    );
}

/// Direct deliveries come from the inbox ledger: a pending row surfaces at the
/// next hook even when the delta window is closed, then is marked delivered.
#[test]
fn turn_check_direct_mentions_surface_from_inbox() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    install_relay_delivery(
        &store,
        [],
        [crate::state::RelayEvent {
            id: "mention-1".to_string(),
            kind: 9,
            pubkey: "pk-chat".to_string(),
            created_at: 120,
            channel_h: "proj".to_string(),
            d_tag: String::new(),
            content: "please review this now".to_string(),
            tags_json: "[[\"p\",\"pk-coder\"]]".to_string(),
        }],
    );
    let newly = store
        .enqueue_inbox(
            "mention-1",
            "pk-coder",
            "pk-chat",
            "proj",
            "please review this now",
            120,
        )
        .unwrap();
    assert!(newly, "first enqueue is newly parked");
    let m = Mutex::new(store);

    let ctx = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", None, 200)
        .expect("direct mention must surface at the next available hook");
    assert!(ctx.contains("please review this now"), "got: {ctx:?}");

    // Drained → marked delivered → not handled-as-pending again.
    let s = m.lock().unwrap();
    assert!(
        s.peek_pending_for_pubkey("pk-coder").unwrap().is_empty(),
        "delivered mention must not remain pending"
    );
    assert!(s.is_event_handled("mention-1", "pk-coder").unwrap());
}
