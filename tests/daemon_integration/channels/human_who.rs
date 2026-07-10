use super::*;

#[test]
fn who_without_agent_anchor_returns_human_fabric_view_with_other_roots() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let channel = unique_session("human-who");
    let other_root = unique_session("human-other");
    let store = Store::open(&home.store_path()).unwrap();
    store.upsert_channel(&channel, &channel, "", "", 1).unwrap();
    store
        .upsert_channel(&other_root, &other_root, "Other work", "", 1)
        .unwrap();
    store
        .upsert_profile("pk-reviewer", "reviewer", "reviewer", "test-host", false, 1)
        .unwrap();
    store
        .upsert_status(&tenex_edge::state::Status {
            pubkey: "pk-reviewer".to_string(),
            session_id: "other-session".to_string(),
            channel_h: other_root.clone(),
            slug: "reviewer".to_string(),
            title: "Reviewing".to_string(),
            activity: String::new(),
            busy: false,
            last_seen: 1,
            updated_at: 1,
            expiration: 9_999_999_999,
        })
        .unwrap();
    drop(store);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "who",
                serde_json::json!({
                    "root": &channel,
                    "human_color": false
                }),
            )
            .await
            .expect("human who should render");

        let human = v["fabric_human"]
            .as_str()
            .expect("human who should include fabric_human");
        assert!(human.starts_with(&format!("{channel}\n\n")), "got: {human}");
        assert!(human.contains("Other workspaces"), "got: {human}");
        assert!(human.contains(&other_root), "got: {human}");
        assert!(human.contains("@reviewer"), "got: {human}");
        assert!(human.contains("1 agent"), "got: {human}");
        assert!(human.contains("Other work"), "got: {human}");
        assert!(!human.contains("<tenex-edge>"), "got: {human}");
        assert!(
            v["fabric"]
                .as_str()
                .is_some_and(|text| text.contains("<tenex-edge>")),
            "daemon should still expose the XML fabric view"
        );
    });

    stop_daemon(&home);
}
