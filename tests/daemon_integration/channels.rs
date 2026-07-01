use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "channels/launch_mentions.rs"]
mod launch_mentions;
#[path = "channels/replies.rs"]
mod replies;
#[path = "channels/session_rooms.rs"]
mod session_rooms;

// ── NIP-29 daemon-owned channels ─────────────────────────────────────────────

/// A valid (throwaway) operator nsec for the local relay — the HUMAN's key.
/// `userNsec` is ONLY used to sign user-prompt events; its pubkey is whitelisted
/// so it's granted admin in every group (signed by `tenexPrivateKey`).
const EXAMPLE_USER_NSEC: &str = "nsec1eulru7a67wt9ndqxv424kmgvd6uyd8defdxh7y9peut28f2p2vhs35m5h4";
/// A valid (throwaway) backend seckey (hex) — distinct from the user's key, per
/// doctrine: `userNsec` is the human, `tenexPrivateKey` is the backend. The
/// backend is the management signer (group create/lock/put-user/etc.) and is
/// automatically an admin of every group it creates.
const EXAMPLE_BACKEND_SEC_HEX: &str =
    "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422";

/// Derive the hex pubkey from an nsec/hex seckey at runtime.
fn pubkey_of(sec: &str) -> String {
    use nostr_sdk::prelude::Keys;
    Keys::parse(sec).unwrap().public_key().to_hex()
}

fn rewrite_config_with_user_nsec(home: &Home) {
    // These tests exercise the per-session-room feature, which is opt-in
    // (`perSessionRooms`, default off) — so enable it here.
    write_config(home, true);
}

/// Write the daemon config, choosing whether human-initiated sessions mint a
/// per-session room (`per_session_rooms`).
fn write_config(home: &Home, per_session_rooms: bool) {
    // NIP-29 ownership/minting needs a NIP-29-aware relay; nak can't do it.
    // The user's pubkey is whitelisted (so it's granted admin in every group),
    // and the backend key signs group management. The two keys are ALWAYS
    // distinct per doctrine: userNsec = human, tenexPrivateKey = backend.
    let user_pk = pubkey_of(EXAMPLE_USER_NSEC);
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [user_pk],
        "backendName": "test-host",
        "relays": [shared_nip29_relay_url()],
        "indexerRelay": shared_nip29_relay_url(),
        "userNsec": EXAMPLE_USER_NSEC,
        "tenexPrivateKey": EXAMPLE_BACKEND_SEC_HEX,
        "perSessionRooms": per_session_rooms,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
}

fn refresh_project_members(project: &str) {
    let _ = tenex_edge::daemon::blocking::call(
        "project_members",
        serde_json::json!({ "project": project }),
    );
}

fn materialize_member_snapshot(home: &Home, project: &str, pubkey: &str) {
    Store::open(&home.store_path())
        .unwrap()
        .replace_channel_members(project, &[pubkey.to_string()], 9_000_000)
        .unwrap();
}

fn unique_session(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}

/// `channels_create` (the launch channel picker's "create new channel" path)
/// must auto-create the parent project group when it doesn't exist on the relay
/// yet. With per-session rooms off (the default), the picker can be the FIRST
/// thing to touch a project, so the parent isn't guaranteed to exist; without
/// the parent-ensure the relay rejects the 9007 with "parent group doesn't
/// exist". Regression for that path.
#[test]
fn channels_create_auto_creates_missing_parent_project() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    // A fresh parent project that has NEVER been opened on the relay.
    let parent = unique_session("freshproj");
    let backend_pk = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);

    let child_h = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let v = c
            .call(
                "channels_create",
                serde_json::json!({
                    "parent": parent,
                    "name": "tester",
                    "agents": [{ "slug": "coder", "backend": backend_pk }],
                    "brief": "",
                }),
            )
            .await
            .expect("channels_create should succeed even when the parent is new");
        v["child_h"].as_str().expect("child_h returned").to_string()
    });

    assert!(!child_h.is_empty(), "channels_create returned a child id");

    // The parent project group was created + locked, so the backend management
    // key is now an admin of it. (Manageability = `is_channel_admin`; the old
    // `is_group_owned` ownership flag no longer exists.)
    let store = Store::open(&home.store_path()).unwrap();
    assert!(
        store.is_channel_admin(&parent, &backend_pk).unwrap(),
        "parent project {parent} should be managed (backend admin) after channels_create created it"
    );

    stop_daemon(&home);
}

/// `channels create` run as an agent (env_session set) with NO `--agent` targets
/// nests the new channel under the creator's CURRENT channel and auto-switches the
/// running session into it. One test covers three behaviors: `--agent` is optional,
/// the parent defaults to the current channel, and the creator auto-switches.
#[test]
fn channels_create_no_agents_nests_under_current_and_auto_switches() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-create");
    let parent = unique_session("currentchan");

    // Start a session pinned to a known current channel (the override wins over
    // any per-session room), kept alive by watching this test process. The channel
    // NAME resolves to an opaque id, so read back the session's actual `channel_h`.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": sid, "cwd": "/tmp", "channel": parent, "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });
    let current_channel = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row")
        .channel_h;

    // Create a child channel as that agent with NO agents and no explicit parent.
    let v = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "channels_create",
            serde_json::json!({
                "name": "subtask",
                "agents": [],
                "env_session": sid,
                "agent": "coder",
                "cwd": "/tmp",
            }),
        )
        .await
        .expect("channels_create with no agents should succeed")
    });

    let child_h = v["child_h"].as_str().expect("child_h returned").to_string();
    assert!(
        v["switched"].as_bool().unwrap_or(false),
        "the creating session should auto-switch into the new channel"
    );
    assert_eq!(
        v["orchestration_event_id"].as_str().unwrap_or("<missing>"),
        "",
        "no --agent targets → no kind:9 orchestration event"
    );

    let store = Store::open(&home.store_path()).unwrap();
    // The new channel nests under the creator's CURRENT channel, not the project root.
    assert_eq!(
        store.channel_parent(&child_h).unwrap().unwrap_or_default(),
        current_channel,
        "new channel should nest under the creator's current channel"
    );
    // The creating session is re-homed onto the new channel.
    let rec = store.get_session(&sid).unwrap().expect("session row");
    assert_eq!(
        rec.channel_h, child_h,
        "session route scope should follow the auto-switch onto the new channel"
    );

    stop_daemon(&home);
}

/// Channel names are unique per parent: re-running `channels create` with a name
/// that already exists under the same parent is a hard ERROR (not a silent dedup),
/// so the agent learns the channel is already there and switches in instead.
#[test]
fn channels_create_errors_when_name_already_exists() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let parent = unique_session("dupproj");
    let backend_pk = pubkey_of(EXAMPLE_BACKEND_SEC_HEX);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let mk = || {
            serde_json::json!({
                "parent": parent,
                "name": "dup",
                "agents": [{ "slug": "coder", "backend": backend_pk }],
            })
        };
        c.call("channels_create", mk())
            .await
            .expect("first create of a fresh name succeeds");
        let err = c
            .call("channels_create", mk())
            .await
            .expect_err("re-creating the same name under the same parent must error");
        assert!(
            format!("{err:?}").contains("already exists"),
            "error must tell the agent the channel already exists, got: {err:?}"
        );
    });

    stop_daemon(&home);
}

/// An orchestration-spawned session (the backend set `TENEX_EDGE_CHANNEL` to add
/// this agent to a task subgroup) joins that group as-is and does NOT mint a
/// child room. Guards the discriminator boundary.
#[test]
fn orchestration_session_uses_existing_group_without_minting() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-orch-1", "cwd": "/tmp", "channel": "issue-42"}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-orch-1")
        .unwrap()
        .expect("session row");
    assert_eq!(
        rec.channel_h, "issue-42",
        "with a channel override the session joins it; it must not mint a child room"
    );

    stop_daemon(&home);
}

/// A user's prompt is published as kind:9 chat into the session's room
/// (operator-signed — the human is speaking, and the operator is the room
/// admin). (Issue #6, increment 3.)
#[test]
fn user_prompt_publishes_kind9_chat_into_room() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-prompt");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": sid, "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    // The room is minted on the relay in the background; wait until the agent is
    // a member (room fully live) before mirroring a prompt into it.
    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row");
    materialize_member_snapshot(&home, &rec.channel_h, &rec.agent_pubkey);
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .map(|s| {
            refresh_project_members(&rec.channel_h);
            s.is_channel_member(&rec.channel_h, &rec.agent_pubkey)
                .unwrap_or(false)
        })
        .unwrap_or(false)),
        "room {} not live in time",
        rec.channel_h
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "user_prompt",
            serde_json::json!({"env_session": sid, "agent": "coder", "cwd": "/tmp", "prompt": "build me a thing"}),
        )
        .await
        .expect("user_prompt");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let msgs = chat_in_channel(&store, &rec.channel_h);
    assert!(
        msgs.iter().any(|m| m.content == "build me a thing"),
        "user prompt should be recorded as chat in room {}; got {:?}",
        rec.channel_h,
        msgs.iter().map(|m| &m.content).collect::<Vec<_>>()
    );

    stop_daemon(&home);
}

/// A prompt Codex dispatches to a subagent it spawned (spawn_agent/
/// multi_agent_v1*) re-fires the same hook on the same session_id as a real
/// human prompt, carrying a `subagent_id`. It must NOT be mirrored as chat —
/// the human didn't say it. (Issue #102.)
#[test]
fn subagent_dispatch_prompt_is_not_mirrored_as_chat() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-subagent-prompt");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": sid, "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row");
    materialize_member_snapshot(&home, &rec.channel_h, &rec.agent_pubkey);
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .map(|s| {
            refresh_project_members(&rec.channel_h);
            s.is_channel_member(&rec.channel_h, &rec.agent_pubkey)
                .unwrap_or(false)
        })
        .unwrap_or(false)),
        "agent should become a room member"
    );

    let resp = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "user_prompt",
            serde_json::json!({
                "env_session": sid,
                "agent": "coder",
                "cwd": "/tmp",
                "prompt": "Please return exactly one random color name as your response.",
                "subagent_id": "019f1cad-51b7-7b32-a340-65e53e932f43",
            }),
        )
        .await
        .expect("user_prompt")
    });
    assert_eq!(
        resp.get("skipped").and_then(|v| v.as_str()),
        Some("subagent dispatch, not human input")
    );

    let store = Store::open(&home.store_path()).unwrap();
    let msgs = chat_in_channel(&store, &rec.channel_h);
    assert!(
        !msgs.iter().any(|m| m.content.contains("random color name")),
        "subagent dispatch prompt must not be mirrored as chat; got {:?}",
        msgs.iter().map(|m| &m.content).collect::<Vec<_>>()
    );

    stop_daemon(&home);
}

/// When the agent finishes a turn (stop hook), its turn output is published as
/// kind:9 chat into the session's room, signed by the agent's DURABLE identity
/// (via keys_for_session → durable fallback). (Issue #6, increment 4.)
#[test]
fn agent_reply_publishes_kind9_chat_into_room() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);
    let sid = unique_session("sess-reply");

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": sid, "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    // Wait for the background mint to make the room live before driving a turn.
    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session(&sid)
        .unwrap()
        .expect("session row");
    materialize_member_snapshot(&home, &rec.channel_h, &rec.agent_pubkey);
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .map(|s| {
            refresh_project_members(&rec.channel_h);
            s.is_channel_member(&rec.channel_h, &rec.agent_pubkey)
                .unwrap_or(false)
        })
        .unwrap_or(false)),
        "room {} not live in time",
        rec.channel_h
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Open a turn so the stop-hook reply publish (gated on was_working) fires.
        c.call("turn_start", serde_json::json!({"session": sid}))
            .await
            .expect("turn_start");
        c.call(
            "turn_end",
            serde_json::json!({"session": sid, "reply": "I fixed the bug in auth.rs"}),
        )
        .await
        .expect("turn_end");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let msgs = chat_in_channel(&store, &rec.channel_h);
    let reply = msgs
        .iter()
        .find(|m| m.content == "I fixed the bug in auth.rs");
    assert!(
        reply.is_some(),
        "agent reply should be chat in room {}; got {:?}",
        rec.channel_h,
        msgs.iter().map(|m| &m.content).collect::<Vec<_>>()
    );
    // The reply is signed by the durable agent identity (the room member), so
    // chat and presence stay on one identity.
    assert_eq!(
        reply.unwrap().pubkey,
        rec.agent_pubkey,
        "agent reply must be signed by the durable agent identity"
    );

    stop_daemon(&home);
}

#[test]
fn session_start_without_tenex_private_key_still_starts_unmanaged() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new(); // default config has NO tenexPrivateKey

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-nogrp", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start must succeed even without tenexPrivateKey");
    });

    // Fail-open: the session runs, but the group stays unmanaged (no ownership).
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-nogrp")
        .unwrap()
        .expect("session row");
    assert!(rec.alive, "session must start even without tenexPrivateKey");
    // Manageability is now "has an admin member" (relay_channel_members, role
    // 'admin'); the old `is_group_owned` ownership flag no longer exists. Without
    // tenexPrivateKey the daemon can't sign group management, so no admin is
    // materialized — the channel stays unmanaged.
    assert!(
        store
            .list_channel_members(&rec.channel_h)
            .unwrap()
            .iter()
            .all(|m| m.role != "admin"),
        "without tenexPrivateKey the daemon must not claim/own the group"
    );

    stop_daemon(&home);
}
