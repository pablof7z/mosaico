use crate::daemon_harness::*;
use mosaico::state::{RecordMessage, Store};

fn cache_message(store: &Store, id: &str, channel: &str, author: &str, body: &str, at: u64) {
    store
        .record_message(&RecordMessage {
            message_id: id.to_string(),
            thread_id: channel.to_string(),
            channel_h: channel.to_string(),
            author_pubkey: author.to_string(),
            body: body.to_string(),
            created_at: at,
            direction: "inbound".to_string(),
            sync_state: "accepted".to_string(),
            native_event_id: Some(id.to_string()),
            error: None,
        })
        .unwrap();
}

#[test]
fn channel_search_reads_cached_messages_across_channels_while_relay_is_wedged() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let relay = WedgeRelay::start();
    let home = Home::with_wedged_relay(&relay.url);
    let store = Store::open(&home.store_path()).unwrap();
    store.upsert_channel("alpha", "alpha", "", "", 1).unwrap();
    store
        .upsert_channel("research-h", "research", "", "alpha", 2)
        .unwrap();
    store.upsert_channel("beta", "beta", "", "", 3).unwrap();
    store
        .upsert_profile("pablo-pk", "Pablo", "Pablo", "", false, 1)
        .unwrap();
    cache_message(
        &store,
        "aaaaaa-new",
        "research-h",
        "pablo-pk",
        "research commit",
        30,
    );
    cache_message(&store, "bbbbbb-mid", "beta", "pablo-pk", "beta commit", 20);
    cache_message(&store, "cccccc-old", "alpha", "pablo-pk", "unrelated", 10);
    drop(store);

    let all = run_cli(
        &home,
        &["channel", "search", "--contains", "COMMIT", "--limit", "2"],
    );
    assert!(
        all.status.success(),
        "search failed while relay was wedged: {}",
        String::from_utf8_lossy(&all.stderr)
    );
    let all = String::from_utf8_lossy(&all.stdout);
    assert!(all.contains("<channel ref=\"/alpha/research\">"), "{all}");
    assert!(all.contains("<channel ref=\"/beta\">"), "{all}");
    assert!(all.contains("id=\"aaaaaa\""), "{all}");
    assert!(all.contains("id=\"bbbbbb\""), "{all}");
    assert!(!all.contains("unrelated"), "{all}");

    let subtree = run_cli(
        &home,
        &[
            "channel",
            "search",
            "--channel",
            "/alpha",
            "--contains",
            "commit",
        ],
    );
    assert!(
        subtree.status.success(),
        "subtree search failed: {}",
        String::from_utf8_lossy(&subtree.stderr)
    );
    let subtree = String::from_utf8_lossy(&subtree.stdout);
    assert!(subtree.contains("research commit"), "{subtree}");
    assert!(!subtree.contains("beta commit"), "{subtree}");

    let mcp = run_cli_stdin(
        &home,
        &["mcp"],
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mosaico.channel_search","arguments":{"contains":["commit"]}}}"#,
    );
    assert!(
        mcp.status.success(),
        "MCP search failed: {}",
        String::from_utf8_lossy(&mcp.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&mcp.stdout).unwrap();
    let result = &response["result"];
    let xml = result["content"][0]["text"].as_str().unwrap();
    assert!(xml.contains("<channel ref=\"/alpha/research\">"), "{xml}");
    assert!(xml.contains("<channel ref=\"/beta\">"), "{xml}");
    assert_eq!(
        result["structuredContent"]["channels"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    stop_daemon(&home);
}
