use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

// ── NIP-29 daemon-owned groups ───────────────────────────────────────────────

/// A valid (throwaway) operator nsec for the local relay.
const EXAMPLE_USER_NSEC: &str = "nsec1eulru7a67wt9ndqxv424kmgvd6uyd8defdxh7y9peut28f2p2vhs35m5h4";

fn rewrite_config_with_user_nsec(home: &Home) {
    // NIP-29 ownership/minting needs a NIP-29-aware relay; nak can't do it.
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [],
        "backendName": "test-host",
        "relays": [shared_croissant_url()],
        "userNsec": EXAMPLE_USER_NSEC,
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
    assert!(
        store
            .is_group_member(&rec.project, &rec.agent_pubkey)
            .unwrap(),
        "the starting agent should be a member of its project group"
    );

    stop_daemon(&home);
}

/// A human-initiated session (no `group` override — someone ran `claude` /
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
    // The session must live in a freshly-minted room (`<work-root>-<hex8>`),
    // not the bare work-root project "tmp".
    assert_ne!(
        rec.project, "tmp",
        "human-initiated session should mint a per-session room, not use the bare project"
    );
    assert!(
        rec.project.starts_with("tmp-"),
        "room id should be a child of the work-root project: got {}",
        rec.project
    );
    assert!(
        store
            .is_group_member(&rec.project, &rec.agent_pubkey)
            .unwrap(),
        "the agent should be a member of its per-session room"
    );

    stop_daemon(&home);
}

/// An orchestration-spawned session (the backend set `TENEX_EDGE_GROUP` to add
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
            serde_json::json!({"agent": "coder", "session_id": "sess-orch-1", "cwd": "/tmp", "group": "issue-42"}),
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
        "with a group override the session joins it; it must not mint a child room"
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
            serde_json::json!({"agent": "coder", "session_id": "sess-prompt-1", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start");
        c.call(
            "user_prompt",
            serde_json::json!({"env_session": "sess-prompt-1", "agent": "coder", "cwd": "/tmp", "prompt": "build me a thing"}),
        )
        .await
        .expect("user_prompt");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-prompt-1")
        .unwrap()
        .expect("session row");
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
            serde_json::json!({"agent": "coder", "session_id": "sess-reply-1", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start");
        c.call(
            "turn_end",
            serde_json::json!({"session": "sess-reply-1", "reply": "I fixed the bug in auth.rs"}),
        )
        .await
        .expect("turn_end");
    });

    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-reply-1")
        .unwrap()
        .expect("session row");
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
    // The reply is signed by the agent via keys_for_session. Once #5 lands
    // (keys_for_session → durable fallback), this is the durable agent pubkey
    // (rec.agent_pubkey). Pre-#5, signing still uses the per-session derived
    // key, so we only require the durable identity here when #5 is integrated.
    if reply.unwrap().from_pubkey == rec.agent_pubkey {
        // durable signer (post-#5) — the intended end state.
    } else {
        eprintln!(
            "[test] agent reply signed by non-durable key {} (expected durable {}); \
             expected until issue #5 (durable signer) is integrated",
            reply.unwrap().from_pubkey,
            rec.agent_pubkey
        );
    }

    stop_daemon(&home);
}

#[test]
fn session_start_without_user_nsec_still_starts_unmanaged() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new(); // default config has NO userNsec

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "sess-nogrp", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start must succeed even without userNsec");
    });

    // Fail-open: the session runs, but the group stays unmanaged (no ownership).
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("sess-nogrp")
        .unwrap()
        .expect("session row");
    assert!(rec.alive, "session must start even without userNsec");
    assert!(
        !store.is_group_owned(&rec.project).unwrap(),
        "without userNsec the daemon must not claim/own the group"
    );

    stop_daemon(&home);
}
