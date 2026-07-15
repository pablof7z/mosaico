use super::*;
use rusqlite::Connection;

#[path = "durable_agent/config.rs"]
mod config;
#[path = "durable_agent/lifecycle.rs"]
mod lifecycle;
use config::{configure_durable_agent, lease_count, read_agent_config, write_agent_config};

#[test]
fn durable_agent_reuses_key_and_rejects_concurrency() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    let relay = rewrite_config_with_nak_relay(&home);
    let slug = "chief-of-staff";
    let durable_pubkey = configure_durable_agent(&home, slug);
    let channel = unique_channel("durable-agent");

    let (first_id, third_id, normal_id, chat_event_id) = rt().block_on(async {
        let mut client = Client::connect_or_spawn().await.expect("connect");
        lifecycle::assert_supervisor_releases_reservations(&home, slug).await;
        let started = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug, "cwd": "/tmp", "channel": channel,
                    "harness": "codex", "harness_session": "durable-native-a",
                }),
            )
            .await
            .expect("durable launch registers session");
        let first = started["pubkey"].as_str().unwrap().to_string();

        let refused = run_cli(&home, &["launch", slug, "--workspace", "tmp"]);
        assert!(!refused.status.success());
        let refused = String::from_utf8_lossy(&refused.stderr);
        assert!(refused.contains("active runtime"), "{refused}");
        assert!(refused.contains(&first), "{refused}");

        let original = read_agent_config(&home, slug);
        let mut flipped = original.clone();
        flipped["perSessionKey"] = serde_json::json!(true);
        write_agent_config(&home, slug, &flipped);
        let fresh_alias_error = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug, "cwd": "/tmp", "channel": channel,
                    "harness": "codex", "harness_session": "fresh-after-mode-flip",
                }),
            )
            .await
            .expect_err("fresh alias cannot bypass a live durable identity");
        assert!(
            fresh_alias_error.to_string().contains("active runtime"),
            "{fresh_alias_error:#}"
        );
        let manual_flip = run_cli(&home, &["launch", slug, "--workspace", "tmp"]);
        assert!(!manual_flip.status.success());
        assert!(
            String::from_utf8_lossy(&manual_flip.stderr).contains("active runtime"),
            "{}",
            String::from_utf8_lossy(&manual_flip.stderr)
        );
        let mode_error = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug, "cwd": "/tmp", "channel": channel,
                    "harness": "codex", "harness_session": "durable-native-a",
                }),
            )
            .await
            .expect_err("durable-to-per-session live mode flip must be rejected");
        assert!(
            mode_error
                .to_string()
                .contains("identity configuration changed"),
            "{mode_error:#}"
        );
        write_agent_config(&home, slug, &original);

        let replacement = nostr_sdk::prelude::Keys::generate();
        let mut rekeyed = original.clone();
        rekeyed["secret_key"] = serde_json::json!(replacement.secret_key().to_secret_hex());
        rekeyed["public_key"] = serde_json::json!(replacement.public_key().to_hex());
        write_agent_config(&home, slug, &rekeyed);
        let key_error = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug, "cwd": "/tmp", "channel": channel,
                    "harness": "codex", "harness_session": "durable-native-a",
                }),
            )
            .await
            .expect_err("live durable key replacement must be rejected");
        assert!(
            key_error
                .to_string()
                .contains("signing configuration no longer reproduces pubkey"),
            "{key_error:#}"
        );
        write_agent_config(&home, slug, &original);

        let normal_slug = "mode-flip-normal";
        mosaico::identity::load_or_create(home.dir.path(), normal_slug, "codex", None, 1).unwrap();
        let normal = start_session(
            &mut client,
            normal_slug,
            Some("normal-native"),
            None,
            &channel,
        )
        .await;
        let mut normal_config = read_agent_config(&home, normal_slug);
        normal_config["perSessionKey"] = serde_json::json!(false);
        write_agent_config(&home, normal_slug, &normal_config);
        let normal_flip = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": normal_slug, "cwd": "/tmp", "channel": channel,
                    "harness": "codex", "harness_session": "normal-native",
                }),
            )
            .await
            .expect_err("per-session-to-durable live mode flip must be rejected");
        assert!(
            normal_flip
                .to_string()
                .contains("identity configuration changed"),
            "{normal_flip:#}"
        );
        normal_config["perSessionKey"] = serde_json::json!(true);
        write_agent_config(&home, normal_slug, &normal_config);

        let error = client
            .call(
                "session_start",
                serde_json::json!({
                    "agent": slug,
                    "cwd": "/tmp",
                    "channel": channel,
                    "harness": "codex",
                    "harness_session": "durable-native-b",
                }),
            )
            .await
            .expect_err("a second live durable-agent session must be rejected");
        assert!(
            error.to_string().contains("active runtime")
                || error.to_string().contains("live session"),
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
        (first, third, normal, chat_event_id)
    });

    assert_eq!(
        first_id, third_id,
        "sequential durable-agent runs reuse their durable pubkey"
    );
    let store = Store::open(&home.store_path()).unwrap();
    for pubkey in [&first_id, &third_id] {
        let session = store.get_session(pubkey).unwrap().unwrap();
        assert_eq!(session.pubkey, durable_pubkey);
        let identity = store.session_identity(pubkey).unwrap().unwrap();
        assert_eq!(identity.display_slug(), slug);
        assert!(identity.durable_agent);
    }
    let normal_session = store.get_session(&normal_id).unwrap().unwrap();
    assert_ne!(normal_session.pubkey, durable_pubkey);
    let db = Connection::open(home.store_path()).unwrap();
    let leases = lease_count(&db, &durable_pubkey);
    assert_eq!(leases, 0, "durable agents never enter handle leasing");
    let normal_leases = lease_count(&db, &normal_session.pubkey);
    assert_eq!(
        normal_leases, 1,
        "rejected mode flip keeps the normal handle"
    );
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
