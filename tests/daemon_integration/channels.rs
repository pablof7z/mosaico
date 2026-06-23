use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

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
    // NIP-29 ownership/minting needs a NIP-29-aware relay; nak can't do it.
    // The user's pubkey is whitelisted (so it's granted admin in every group),
    // and the backend key signs group management. The two keys are ALWAYS
    // distinct per doctrine: userNsec = human, tenexPrivateKey = backend.
    let user_pk = pubkey_of(EXAMPLE_USER_NSEC);
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [user_pk],
        "backendName": "test-host",
        "relays": [shared_croissant_url()],
        "userNsec": EXAMPLE_USER_NSEC,
        "tenexPrivateKey": EXAMPLE_BACKEND_SEC_HEX,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
}

#[test]
fn session_start_with_user_nsec_owns_group_and_adds_member() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home); // daemon reads this at spawn

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            // Supply a live watch_pid (this test process) so the daemon engine's
            // pid-watcher sees a running process and the session stays alive.
            // Real hooks always send the harness pid; omitting it lets the engine
            // self-terminate (no live process to watch) and race the store read.
            serde_json::json!({"agent": "coder", "session_id": "sess-grp-1", "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    // ensure_group_and_membership runs (and writes the cache) before session_start
    // returns, so by now the store records ownership + membership for this project.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-grp-1")
        .unwrap()
        .expect("session row");
    assert!(rec.alive);
    assert!(
        store.is_group_owned(&rec.project).unwrap(),
        "project group should be owned after session_start with userNsec"
    );
    // Room membership is established by the background mint; wait for it.
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .unwrap()
        .is_group_member(&rec.project, &rec.agent_pubkey)
        .unwrap()),
        "the starting agent should be a member of its project group"
    );

    stop_daemon(&home);
}

/// A human-initiated session (no `channel` override — someone ran `claude` /
/// `tenex-edge launch` directly) mints its OWN per-session room: a child
/// subgroup of the work-root project, not the bare project. (Issue #6.)
#[test]
fn human_initiated_session_mints_per_session_room() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-room-1", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-room-1")
        .unwrap()
        .expect("session row");
    // The session must live in a freshly-minted room (`session-<hash>`), not the
    // bare work-root project "tmp".
    assert_ne!(
        rec.project, "tmp",
        "human-initiated session should mint a per-session room, not use the bare project"
    );
    assert!(
        rec.project.starts_with("session-"),
        "room id should be a project-agnostic session room: got {}",
        rec.project
    );
    // Membership lands via the background mint.
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .unwrap()
        .is_group_member(&rec.project, &rec.agent_pubkey)
        .unwrap()),
        "the agent should be a member of its per-session room"
    );

    stop_daemon(&home);
}

/// An opencode-style human session — no harness/resume id, only a watched pid —
/// still mints a per-session room (anchored on the pid), rather than being left
/// in the bare project. (Issue #6; regression for the codex-flagged opencode gap.)
#[test]
fn opencode_style_session_without_id_mints_room_via_pid() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // No session_id / resume_id — exactly what the opencode plugin sends.
        c.call(
            "session_start",
            serde_json::json!({"agent": "opencoder", "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    let store = Store::open(&home.store_path()).unwrap();
    // The opencode session has no harness id; find it by agent slug.
    let rec = store
        .list_alive_sessions()
        .unwrap()
        .into_iter()
        .find(|r| r.agent_slug == "opencoder")
        .expect("opencode session row");
    assert!(
        rec.project.starts_with("session-"),
        "opencode session must mint a per-session room, not stay in the bare project: got {}",
        rec.project
    );
    assert!(
        store.is_session_room(&rec.project).unwrap(),
        "minted group must be flagged as a per-session room"
    );

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
        rec.project, "issue-42",
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

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-prompt-1", "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    // The room is minted on the relay in the background; wait until the agent is
    // a member (room fully live) before mirroring a prompt into it.
    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session("sess-prompt-1")
        .unwrap()
        .expect("session row");
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .unwrap()
        .is_group_member(&rec.project, &rec.agent_pubkey)
        .unwrap()),
        "room {} not live in time",
        rec.project
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "user_prompt",
            serde_json::json!({"env_session": "sess-prompt-1", "agent": "coder", "cwd": "/tmp", "prompt": "build me a thing"}),
        )
        .await
        .expect("user_prompt");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let msgs = store
        .list_chat_messages(&rec.project, 0, None, 0, false)
        .unwrap();
    assert!(
        msgs.iter().any(|m| m.body == "build me a thing"),
        "user prompt should be recorded as chat in room {}; got {:?}",
        rec.project,
        msgs.iter().map(|m| &m.body).collect::<Vec<_>>()
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

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-reply-1", "cwd": "/tmp", "watch_pid": std::process::id()}),
        )
        .await
        .expect("session_start");
    });

    // Wait for the background mint to make the room live before driving a turn.
    let rec = Store::open(&home.store_path())
        .unwrap()
        .get_session("sess-reply-1")
        .unwrap()
        .expect("session row");
    assert!(
        wait_until(std::time::Duration::from_secs(20), || Store::open(
            &home.store_path()
        )
        .unwrap()
        .is_group_member(&rec.project, &rec.agent_pubkey)
        .unwrap()),
        "room {} not live in time",
        rec.project
    );

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Open a turn so the stop-hook reply publish (gated on was_working) fires.
        c.call("turn_start", serde_json::json!({"session": "sess-reply-1"}))
            .await
            .expect("turn_start");
        c.call(
            "turn_end",
            serde_json::json!({"session": "sess-reply-1", "reply": "I fixed the bug in auth.rs"}),
        )
        .await
        .expect("turn_end");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let msgs = store
        .list_chat_messages(&rec.project, 0, None, 0, false)
        .unwrap();
    let reply = msgs.iter().find(|m| m.body == "I fixed the bug in auth.rs");
    assert!(
        reply.is_some(),
        "agent reply should be chat in room {}; got {:?}",
        rec.project,
        msgs.iter().map(|m| &m.body).collect::<Vec<_>>()
    );
    // The reply is signed by the durable agent identity (the room member), so
    // chat and presence stay on one identity.
    assert_eq!(
        reply.unwrap().from_pubkey,
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
    assert!(
        !store.is_group_owned(&rec.project).unwrap(),
        "without tenexPrivateKey the daemon must not claim/own the group"
    );

    stop_daemon(&home);
}
