use super::{rewrite_config_with_user_nsec, unique_session, write_config};
use crate::daemon_harness::{
    hook_session_start, only_session_route, pubkey_for_harness_session, rt, stop_daemon,
    wait_for_exact_relay_groups, wait_for_relay_group_parent, wait_until, Home, ENV_LOCK,
};
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use nostr::Keys;
use std::collections::BTreeSet;
use std::time::Duration;

#[path = "session_rooms/profile.rs"]
mod profile;
#[path = "session_rooms/root_selection.rs"]
mod root_selection;

fn test_log(home: &Home) -> String {
    std::fs::read_to_string(home.dir.path().join("daemon.log")).unwrap_or_else(|e| format!("<{e}>"))
}

fn wait_for_root_observation(home: &Home, root: &str) {
    let path = format!("#{root}");
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            mosaico::daemon::blocking::call(
                "channel_members",
                serde_json::json!({ "channel": path }),
            )
            .is_ok()
        }),
        "NMP did not deliver root {root}; daemon_log={}",
        test_log(home)
    );
}

fn collect_channel_paths(value: &serde_json::Value, paths: &mut Vec<String>) {
    if let Some(path) = value.get("path").and_then(serde_json::Value::as_str) {
        paths.push(path.to_string());
    }
    if let Some(array) = value.as_array() {
        for item in array {
            collect_channel_paths(item, paths);
        }
    }
    if let Some(object) = value.as_object() {
        for child in object.values() {
            collect_channel_paths(child, paths);
        }
    }
}

fn observed_member_path(root: &str, pubkey: &str) -> Option<String> {
    let list =
        mosaico::daemon::blocking::call("channel_list", serde_json::json!({ "recursive": true }))
            .ok()?;
    let mut paths = Vec::new();
    collect_channel_paths(&list, &mut paths);
    paths.into_iter().find(|path| {
        (path == &format!("#{root}") || path.starts_with(&format!("#{root}/")))
            && mosaico::daemon::blocking::call(
                "channel_members",
                serde_json::json!({ "channel": path }),
            )
            .ok()
            .and_then(|members| members["members"].as_array().cloned())
            .is_some_and(|members| members.iter().any(|member| member["pubkey"] == pubkey))
    })
}

fn wait_for_channel_member(home: &Home, root: &str, pubkey: &str) -> String {
    let mut path = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            path = observed_member_path(root, pubkey);
            path.is_some()
        }),
        "member {pubkey} was absent from every NMP-delivered channel under #{root}; daemon_log={}",
        test_log(home)
    );
    path.expect("asserted above")
}

fn wait_for_session_room(channel_h: &str, parent: &str, pubkey: &str) {
    let relay = crate::daemon_harness::shared_nip29_relay_url();
    wait_for_relay_group_parent(&relay, channel_h, parent, Duration::from_secs(25));
    wait_for_exact_relay_groups(
        &relay,
        pubkey,
        &BTreeSet::from([channel_h.to_string()]),
        Duration::from_secs(25),
    );
}

/// e2e: a human-initiated session's first turn gets the channel-hierarchy
/// context block, rendered through the real daemon.
#[test]
fn first_turn_injects_channel_context_block() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    let (channel_h, agent_pubkey) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "sess-ctx-1", "cwd": "/tmp", "watch_pid": std::process::id()}), "claude-code"),
        )
        .await
        .expect("session_start");
        let store = Store::open(&home.store_path()).unwrap();
        let pubkey = pubkey_for_harness_session(&store, "claude-code", "sess-ctx-1").unwrap();
        let rec = store.get_session(&pubkey).unwrap().expect("session row");
        (only_session_route(&store, &rec.pubkey), rec.pubkey)
    });
    wait_for_session_room(&channel_h, "tmp", &agent_pubkey);

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
    // Self identity exposes the public handle, immutable launch workspace, and
    // host without repeating internal session identifiers.
    assert!(
        ctx.contains("<self name=\"@")
            && ctx.contains("host=\"test-host\"")
            && ctx.contains("workspace=\"tmp\""),
        "no canonical self block: {ctx}"
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
        wait_for_root_observation(&home, "tmp");
        c.call(
            "channel_add_member",
            serde_json::json!({"channel": "#tmp", "pubkey": remote_pk, "session": &pubkey}),
        )
        .await
        .expect("channel_add_member profiled member");
        let member_path = wait_for_channel_member(&home, "tmp", &remote_pk);
        assert_eq!(member_path, "#tmp");
        let members = c
            .call("channel_members", serde_json::json!({"channel": "#tmp"}))
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
        members.to_string()
    });

    assert!(
        ctx.contains(&remote_handle),
        "kind:0 profile should resolve: {ctx}"
    );
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
    let channel_h = only_session_route(&store, &rec.pubkey);
    wait_for_session_room(&channel_h, "tmp", &rec.pubkey);

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
    let channel_h = only_session_route(&store, &rec.pubkey);
    assert_ne!(
        channel_h, "tmp",
        "human-initiated session should mint a per-session room, not use the bare channel"
    );
    assert!(
        channel_h.starts_with("session-"),
        "room id should be channel-agnostic: got {}",
        channel_h
    );
    // Channel hierarchy labels come from the current NMP group projection,
    // while the locally generated opaque id remains channel-agnostic.

    wait_for_session_room(&channel_h, "tmp", &rec.pubkey);

    stop_daemon(&home);
}
