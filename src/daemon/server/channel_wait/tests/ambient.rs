use super::*;

#[tokio::test]
async fn ambient_rpc_returns_first_new_chat_from_any_joined_channel() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state);
    let waiting = {
        let state = state.clone();
        tokio::spawn(async move {
            rpc_channel_wait(
                &state,
                &serde_json::json!({
                    "session": SELF_PUBKEY,
                    "timeout_secs": 2,
                }),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;
    deliver_chat(&state, "new-chat", Y_CHANNEL, "peer-pk", "hello", None);
    state.emit_tail(TailEvent::Msg {
        ts: 10,
        channel: Y_CHANNEL.into(),
        from: "peer".into(),
        to: "channel-chat".into(),
        body: "hello".into(),
    });

    let result = waiting.await.unwrap().unwrap();
    assert_eq!(result["outcome"], "message");
    assert_eq!(result["message"]["event_id"], "new-chat");
    assert_eq!(result["message"]["channel"], "#root/y");
    assert!(result["message"].get("channel_ref").is_none());
}

#[tokio::test]
async fn timeout_is_a_normal_structured_outcome() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state);
    let result = rpc_channel_wait(
        &state,
        &serde_json::json!({
            "session": SELF_PUBKEY,
            "timeout_secs": 1,
            "channels": ["#root/x"],
        }),
    )
    .await
    .unwrap();

    assert_eq!(result["outcome"], "timeout");
    assert_eq!(result["timeout_secs"], 1);
    assert_eq!(result["channels"], serde_json::json!(["#root/x"]));
    let rec = state
        .with_store(|store| store.get_session(SELF_PUBKEY))
        .unwrap()
        .unwrap();
    assert!(!state.has_matching_active_wait(&rec, X_CHANNEL, &["peer-pk".into()]));
}
