use super::*;

/// Replacing a demand briefly installs a level without chat. A wait that drains
/// during that window must not consume the arrival, or the message is lost: its
/// arrival is never recorded again when the level returns.
#[tokio::test]
async fn chat_briefly_absent_from_the_view_is_delivered_when_it_returns() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    let mut cursor = state
        .with_store(|store| store.latest_message_arrival_sequence())
        .unwrap();
    deliver_chat(&state, "late", X_CHANNEL, "peer-pk", "hello", None);
    state.with_store(|store| store.install_test_relay_delivery(TestRelayDelivery::new()));
    let filter = AuthorFilter::from_params(&WaitParams::default()).unwrap();
    let drain = |cursor: &mut u64| {
        drain_matching(
            &state,
            cursor,
            &[X_CHANNEL.into()],
            None,
            &filter,
            &own_pubkeys(&rec),
            &state.backend_pubkey(),
        )
        .unwrap()
    };

    assert!(drain(&mut cursor).is_none());
    deliver_chat(&state, "late", X_CHANNEL, "peer-pk", "hello", None);
    assert_eq!(drain(&mut cursor).unwrap().message_id, "late");
}
