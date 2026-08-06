use super::*;

#[tokio::test]
async fn wait_is_scope_author_and_generation_fenced_then_removed_on_cancel() {
    let state = DaemonState::new_for_test().await;
    let rec = seed_session(&state);
    let waiting = {
        let state = state.clone();
        tokio::spawn(async move {
            rpc_channel_wait(
                &state,
                &serde_json::json!({
                    "session": SELF_PUBKEY,
                    "timeout_secs": 30,
                    "channels": ["#root/x"],
                    "from_pubkeys": ["peer-pk"],
                }),
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await;

    assert!(state.has_matching_active_wait(&rec, X_CHANNEL, &["peer-pk".into()]));
    assert!(!state.has_matching_active_wait(&rec, Y_CHANNEL, &["peer-pk".into()]));
    assert!(!state.has_matching_active_wait(&rec, X_CHANNEL, &["other-pk".into()]));
    let mut later_generation = rec.clone();
    later_generation.runtime_generation += 1;
    assert!(!state.has_matching_active_wait(&later_generation, X_CHANNEL, &["peer-pk".into()]));

    waiting.abort();
    let _ = waiting.await;
    assert!(!state.has_matching_active_wait(&rec, X_CHANNEL, &["peer-pk".into()]));
}
