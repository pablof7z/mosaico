use super::*;

#[tokio::test]
async fn management_channel_labels_are_public_paths_or_generic() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.install_test_nmp_group_delivery(crate::state::TestGroupDelivery::new([
                crate::state::TestGroup::new("root").metadata("general", "", "", 1),
                crate::state::TestGroup::new("opaque-child").metadata("review", "", "root", 2),
            ]));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap();

    assert_eq!(channel_label(&state, "opaque-child"), "#root/review");
    assert_eq!(
        channel_label(&state, "unknown-internal-id"),
        "a channel with unavailable public path"
    );
}
