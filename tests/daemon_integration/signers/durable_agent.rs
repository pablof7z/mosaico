use super::*;
use rusqlite::Connection;

fn configure_durable_agent(home: &Home, slug: &str) -> String {
    let identity = tenex_edge::identity::load_or_create(home.dir.path(), slug, 1).unwrap();
    let path = home.dir.path().join("agents").join(format!("{slug}.json"));
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["perSessionKey"] = serde_json::json!(false);
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();
    identity.pubkey_hex()
}

#[test]
fn durable_agent_reuses_key_rejects_concurrency_and_never_becomes_resumable() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    let relay = rewrite_config_with_nak_relay(&home);
    let slug = "chief-of-staff";
    let durable_pubkey = configure_durable_agent(&home, slug);
    let channel = unique_channel("durable-agent");

    let (first_id, third_id, chat_event_id) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        let first =
            start_session(&mut client, slug, Some("durable-native-a"), None, &channel).await;

        let error = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug,
                    "cwd": "/tmp",
                    "channel": channel,
                    "harness": "codex",
                    "session_id": "durable-native-b",
                }),
            )
            .await
            .expect_err("a second live durable-agent session must be rejected");
        assert!(
            error.to_string().contains("already has a live session"),
            "unexpected rejection: {error:#}"
        );

        let sent = client
            .call(
                "channel_send",
                serde_json::json!({
                    "session": &first,
                    "channel": &channel,
                    "message": "durable signer check",
                }),
            )
            .await
            .expect("send as durable agent");
        let chat_event_id = sent["event_id"].as_str().unwrap().to_string();

        client
            .call("session_end", serde_json::json!({ "session": first }))
            .await
            .expect("end first durable session");
        let third =
            start_session(&mut client, slug, Some("durable-native-a"), None, &channel).await;
        (first, third, chat_event_id)
    });

    assert_ne!(
        first_id, third_id,
        "sequential durable-agent runs always start fresh"
    );
    let store = Store::open(&home.store_path()).unwrap();
    for session_id in [&first_id, &third_id] {
        let session = store.get_session(session_id).unwrap().unwrap();
        assert_eq!(session.agent_pubkey, durable_pubkey);
        assert!(session.resume_id.is_empty());
        let identity = store
            .session_identity_for_session(session_id)
            .unwrap()
            .unwrap();
        assert_eq!(identity.display_slug(), slug);
        assert!(identity.durable_agent);
    }
    assert!(store
        .list_resumable_sessions(100)
        .unwrap()
        .iter()
        .all(|session| session.agent_pubkey != durable_pubkey));

    let db = Connection::open(home.store_path()).unwrap();
    let leases: u64 = db
        .query_row(
            "SELECT COUNT(*) FROM handle_leases WHERE pubkey=?1",
            [&durable_pubkey],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leases, 0, "durable agents never enter handle leasing");
    let current: (String, bool) = db
        .query_row(
            "SELECT session_id, live FROM durable_agent_sessions WHERE pubkey=?1",
            [&durable_pubkey],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(current, (third_id, true));

    assert!(
        wait_until(std::time::Duration::from_secs(20), || {
            relay::kind0_name_for_author(&relay, &durable_pubkey).as_deref() == Some(slug)
        }),
        "durable kind:0 must use the bare agent slug"
    );
    assert_eq!(
        relay::event_author(&relay, &chat_event_id).as_deref(),
        Some(durable_pubkey.as_str()),
        "command-triggered chat must use the durable signer"
    );
    stop_daemon(&home);
}
