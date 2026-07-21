use super::{
    materialize_member_snapshot, refresh_channel_members, rewrite_config_with_user_nsec,
    unique_session, write_config,
};
use crate::daemon_harness::{
    hook_session_start, pubkey_for_harness_session, rt, stop_daemon, wait_until, Home, ENV_LOCK,
};
use mosaico::daemon::client::Client;
use mosaico::state::{Status, Store};
use nostr_sdk::prelude::Keys;

#[path = "session_rooms/profile.rs"]
mod profile;
#[path = "session_rooms/root_selection.rs"]
mod root_selection;

fn test_log(home: &Home) -> String {
    std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_else(|e| format!("<{e}>"))
}

fn wait_for_channel_metadata(home: &Home, channel: &str) {
    assert!(
        wait_until(std::time::Duration::from_secs(25), || Store::open(
            &home.store_path()
        )
        .map(|s| s.get_channel(channel).unwrap_or(None).is_some())
        .unwrap_or(false)),
        "channel {channel} metadata did not materialize; daemon_log={}",
        test_log(home)
    );
}

fn wait_for_channel_parent(home: &Home, channel: &str, parent: &str) {
    assert!(
        wait_until(std::time::Duration::from_secs(25), || Store::open(
            &home.store_path()
        )
        .map(|s| s.channel_parent(channel).unwrap_or(None).as_deref() == Some(parent))
        .unwrap_or(false)),
        "channel {channel} parent {parent} did not materialize; daemon_log={}",
        test_log(home)
    );
}

fn wait_for_channel_member(home: &Home, channel: &str, pubkey: &str) {
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            refresh_channel_members(channel);
            Store::open(&home.store_path())
                .map(|s| s.is_channel_member(channel, pubkey).unwrap_or(false))
                .unwrap_or(false)
        }),
        "member {pubkey} did not materialize in {channel}; daemon_log={}",
        test_log(home)
    );
}

/// e2e: a human-initiated session's first turn gets the channel-hierarchy
/// context block, rendered through the real daemon.
#[test]
fn first_turn_injects_channel_context_block() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    let (channel, agent_pubkey) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "sess-ctx-1", "cwd": "/tmp", "watch_pid": std::process::id()}), "claude-code"),
        )
        .await
        .expect("session_start");
        let store = Store::open(&home.store_path()).unwrap();
        let pubkey = pubkey_for_harness_session(&store, "claude-code", "sess-ctx-1").unwrap();
        let rec = store
            .get_session(&pubkey)
            .unwrap()
            .expect("session row");
        (rec.channel_h, rec.pubkey)
    });
    wait_for_channel_parent(&home, &channel, "tmp");
    wait_for_channel_member(&home, &channel, &agent_pubkey);

    let ctx = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "turn_start",
                serde_json::json!({
                    "harness_session": "sess-ctx-1",
                    "harness": "claude-code"
                }),
            )
            .await
            .expect("turn_start");
        v.get("context")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string()
    });

    assert!(ctx.contains("<mosaico>"), "context was: {ctx}");
    assert!(
        !ctx.contains("[session"),
        "must not expose a session code; context was: {ctx}"
    );
    assert!(
        !ctx.contains("(session "),
        "must not repeat the raw session id; context was: {ctx}"
    );
    assert!(ctx.contains("<channel "), "context was: {ctx}");
    // Self identity should distinguish the stable agent from the session and backend.
    assert!(
        ctx.contains("Agent: coder · Session: @") && ctx.contains("-coder · Backend: test-host"),
        "no self line: {ctx}"
    );

    stop_daemon(&home);
}

#[test]
fn first_turn_resolves_member_profiles_from_kind0() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);
    let sid = unique_session("sess-member-profile");
    let remote = Keys::generate();
    let remote_pk = remote.public_key().to_hex();
    let remote_name = "willow-echo-042";
    let remote_agent_slug = "reviewer";
    let remote_handle =
        mosaico::idref::session_handle_from_profile_name(remote_name, remote_agent_slug);

    let ctx = rt().block_on(async {
        profile::publish_profile(&remote, remote_name, remote_agent_slug).await;
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let started = c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": &sid, "cwd": "/tmp", "watch_pid": std::process::id()}), "claude-code"),
        )
        .await
        .expect("session_start");
        let pubkey = started["pubkey"].as_str().unwrap().to_string();
        wait_for_channel_metadata(&home, "tmp");
        c.call(
            "channel_add_member",
            serde_json::json!({"channel": "tmp", "pubkey": remote_pk, "session": &pubkey}),
        )
        .await
        .expect("channel_add_member profiled member");
        wait_for_channel_member(&home, "tmp", &remote_pk);
        let now = mosaico::util::now_secs();
        Store::open(&home.store_path())
            .unwrap()
            .upsert_status(&Status {
                pubkey: remote_pk.clone(),
                channel_h: "tmp".into(),
                slug: String::new(),
                title: "Reviewing".into(),
                activity: String::new(),
                state: mosaico::session_state::SessionState::Idle,
                last_seen: now,
                updated_at: now,
                expiration: now + 90,
            })
            .unwrap();
        let members = c
            .call("channel_members", serde_json::json!({"channel": "tmp"}))
            .await
            .expect("channel_members");
        assert!(
            members["members"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["pubkey"] == remote_pk && m["slug"] == remote_handle),
            "channel_members should resolve kind:0 slugs: {members}"
        );
        let v = c
            .call(
                "turn_start",
                serde_json::json!({
                    "harness_session": sid,
                    "harness": "claude-code"
                }),
            )
            .await
            .expect("turn_start");
        v.get("context")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string()
    });

    let want = format!("ref=\"@{remote_handle}\" state=\"idle\" status=\"Reviewing\"");
    assert!(ctx.contains(&want), "kind:0 profile should resolve: {ctx}");
    assert!(
        !ctx.contains(&format!("@{}", &remote_pk[..8])),
        "raw pubkey leaked: {ctx}"
    );

    stop_daemon(&home);
}

#[test]
fn session_start_with_user_nsec_owns_group_and_adds_member() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "sess-grp-1", "cwd": "/tmp", "watch_pid": std::process::id()}), "claude-code"),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session(&pubkey_for_harness_session(&store, "claude-code", "sess-grp-1").unwrap())
        .unwrap()
        .expect("session row");
    assert!(rec.is_running());
    // The minted session room's parent is the work-root channel. (Parent
    // now lives in `relay_channels`; `session_room_parent` was renamed to
    // `channel_parent`.)
    wait_for_channel_parent(&home, &rec.channel_h, "tmp");

    stop_daemon(&home);
}

/// Human-initiated sessions with per-session rooms enabled mint child rooms
/// under the work-root channel.
#[test]
fn human_initiated_session_mints_per_session_room() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-room");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(
                serde_json::json!({"agent": "coder", "harness_session": sid, "cwd": "/tmp"}),
                "claude-code",
            ),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&store, "claude-code", &sid).unwrap();
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    assert_ne!(
        rec.channel_h, "tmp",
        "human-initiated session should mint a per-session room, not use the bare channel"
    );
    assert!(
        rec.channel_h.starts_with("session-"),
        "room id should be channel-agnostic: got {}",
        rec.channel_h
    );
    // removed: `channel_breadcrumb` no longer exists — channel hierarchy labels
    // are derived from `relay_channels` (name/parent), not a breadcrumb reader.

    materialize_member_snapshot(&home, &rec.channel_h, &rec.pubkey);
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .map(|s| {
            refresh_channel_members(&rec.channel_h);
            s.is_channel_member(&rec.channel_h, &rec.pubkey)
                .unwrap_or(false)
        })
        .unwrap_or(false)),
        "the agent should be a member of its per-session room"
    );

    stop_daemon(&home);
}
