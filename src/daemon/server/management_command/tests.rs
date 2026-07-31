use super::*;

#[tokio::test]
async fn management_channel_labels_are_public_paths_or_generic() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.upsert_channel("root", "general", "", "", 1)?;
            store.upsert_channel("opaque-child", "review", "", "root", 2)
        })
        .unwrap();

    assert_eq!(channel_label(&state, "opaque-child"), "#root/review");
    assert_eq!(
        channel_label(&state, "unknown-internal-id"),
        "a channel with unavailable public path"
    );
}
