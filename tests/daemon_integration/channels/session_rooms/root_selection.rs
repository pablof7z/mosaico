//! Per-session-room vs work-root channel selection at session start. Split out of
//! `session_rooms.rs` to keep that file under its LOC baseline.
use super::super::{rewrite_config_with_user_nsec, unique_session, write_config};
use super::observed_member_path;
use crate::daemon_harness::{
    hook_session_start, only_session_route, pubkey_for_harness_session, rt, stop_daemon,
    wait_for_exact_relay_groups, wait_until, Home, ENV_LOCK,
};
use mosaico::daemon::client::Client;
use mosaico::state::Store;
use std::collections::BTreeSet;
use std::time::Duration;

/// With per-session rooms disabled, a human-initiated session uses the work-root
/// root channel.
#[test]
fn human_initiated_session_uses_root_when_per_session_rooms_disabled() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    write_config(&home, false);
    let sid = unique_session("sess-noroom");

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
    assert_eq!(
        channel_h, "tmp",
        "with per-session rooms disabled, the session should use the root channel"
    );
    assert!(
        !channel_h.starts_with("session-"),
        "no per-session room should be minted: got {}",
        channel_h
    );
    let mut observed_path = None;
    assert!(
        wait_until(std::time::Duration::from_secs(25), || {
            observed_path = observed_member_path("tmp", &rec.pubkey);
            observed_path.as_deref() == Some("#tmp")
        }),
        "NMP's delivered view should place the session in the work-root itself; got {observed_path:?}"
    );
    wait_for_exact_relay_groups(
        &crate::daemon_harness::shared_nip29_relay_url(),
        &rec.pubkey,
        &BTreeSet::from(["tmp".to_string()]),
        Duration::from_secs(25),
    );

    stop_daemon(&home);
}

/// Opencode-style human sessions have no harness/resume id, so the room anchor
/// falls back to the watched pid.
#[test]
fn opencode_style_session_without_id_mints_room_via_pid() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "opencoder", "cwd": "/tmp", "watch_pid": std::process::id()}), "opencode"),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .list_running_sessions()
        .unwrap()
        .into_iter()
        .find(|r| r.agent_slug == "opencoder")
        .expect("opencode session row");
    let channel_h = only_session_route(&store, &rec.pubkey);
    assert!(
        channel_h.starts_with("session-"),
        "opencode session must mint a per-session room: got {}",
        channel_h
    );
    super::wait_for_session_room(&channel_h, "tmp", &rec.pubkey);

    stop_daemon(&home);
}
