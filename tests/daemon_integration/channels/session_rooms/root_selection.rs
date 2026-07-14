//! Per-session-room vs work-root channel selection at session start. Split out of
//! `session_rooms.rs` to keep that file under its LOC baseline.
use super::super::{rewrite_config_with_user_nsec, unique_session, write_config};
use crate::daemon_harness::{
    pubkey_for_harness_session, rt, stop_daemon, wait_until, Home, ENV_LOCK,
};
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

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
            serde_json::json!({"agent": "coder", "harness_session": sid, "cwd": "/tmp"}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = pubkey_for_harness_session(&store, "claude-code", &sid).unwrap();
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    assert_eq!(
        rec.channel_h, "tmp",
        "with per-session rooms disabled, the session should use the root channel"
    );
    assert!(
        !rec.channel_h.starts_with("session-"),
        "no per-session room should be minted: got {}",
        rec.channel_h
    );
    // A session room is a channel with a non-empty parent; the work-root channel
    // is a root channel. (`is_session_room` was removed; the distinction is
    // `is_root_channel`.)
    assert!(
        wait_until(std::time::Duration::from_secs(25), || Store::open(
            &home.store_path()
        )
        .map(|s| s.is_root_channel(&rec.channel_h).unwrap_or(false))
        .unwrap_or(false)),
        "the work-root channel is not a session room"
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
            serde_json::json!({"agent": "opencoder", "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .list_alive_sessions()
        .unwrap()
        .into_iter()
        .find(|r| r.agent_slug == "opencoder")
        .expect("opencode session row");
    assert!(
        rec.channel_h.starts_with("session-"),
        "opencode session must mint a per-session room: got {}",
        rec.channel_h
    );
    // A minted session room is a non-root channel (it has a parent channel).
    // (`is_session_room` was removed; the distinction is `!is_root_channel`.)
    // The room's relay_channels row can materialize BEFORE its parent link (the
    // 39000 metadata), during which it would transiently look root — so wait for
    // the parent (non-root) to materialize rather than reading it once.
    assert!(
        wait_until(std::time::Duration::from_secs(25), || Store::open(
            &home.store_path()
        )
        .map(|s| s.get_channel(&rec.channel_h).unwrap_or(None).is_some()
            && !s.is_root_channel(&rec.channel_h).unwrap_or(true))
        .unwrap_or(false)),
        "minted group must be a per-session room (non-root channel)"
    );

    stop_daemon(&home);
}
