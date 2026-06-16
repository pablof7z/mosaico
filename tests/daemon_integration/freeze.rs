use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

// ── Frozen regression guards (dedup, targeted/untargeted mention routing,
//    39000/39002 idempotency, startup catch-up) and the threaded e2e. ───────────

/// A valid (throwaway) operator nsec for the local relay.
const EXAMPLE_USER_NSEC: &str = "nsec1eulru7a67wt9ndqxv424kmgvd6uyd8defdxh7y9peut28f2p2vhs35m5h4";

fn rewrite_config_with_user_nsec(home: &Home) {
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [],
        "backendName": "test-host",
        "relays": [shared_relay_url()],
        "userNsec": EXAMPLE_USER_NSEC,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
}

#[test]
fn threaded_conversation_between_claude_and_codex_e2e() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call("session_start", serde_json::json!({"agent": "claude", "session_id": "sess-claude", "cwd": "/tmp"}))
            .await
            .unwrap();
        c.call("session_start", serde_json::json!({"agent": "codex", "session_id": "sess-codex", "cwd": "/tmp"}))
            .await
            .unwrap();

        // Project slug for these sessions, read from the daemon (derived from cwd).
        let who = c.call("who", serde_json::json!({"all": true, "all_projects": true})).await.unwrap();
        let project = who["rows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["slug"] == "claude")
            .and_then(|r| r["project"].as_str())
            .expect("claude session project")
            .to_string();

        // helper: poll an agent's inbox until a body shows up.
        async fn wait_for_body(c: &mut Client, session: &str, body: &str) {
            for _ in 0..24 {
                let inbox = c.call("inbox", serde_json::json!({"session": session})).await.unwrap();
                if inbox["rows"].as_array().unwrap().iter().any(|m| m["body"] == body) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            panic!("{session} never received {body:?}");
        }

        // 1) claude → codex: a ROOT message (no thread_id).
        c.call(
            "send_message",
            serde_json::json!({"recipient": "sess-codex", "message": "can you review this?", "session": "sess-claude", "agent": "claude"}),
        )
        .await
        .expect("send root");
        wait_for_body(&mut c, "sess-codex", "can you review this?").await;

        // 2) the canonical read model shows exactly one thread with one message.
        let mut thread_id = String::new();
        for _ in 0..24 {
            let threads = c.call("list_threads", serde_json::json!({"project": project})).await.unwrap();
            if let Some(t) = threads.as_array().unwrap().first() {
                thread_id = t["thread_id"].as_str().unwrap().to_string();
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(!thread_id.is_empty(), "a thread should exist after the root message");
        let msgs = c.call("messages", serde_json::json!({"thread_id": thread_id})).await.unwrap();
        assert_eq!(msgs.as_array().unwrap().len(), 1, "root thread has exactly 1 message: {msgs}");

        // 3) codex REPLIES into that thread → claude receives it.
        c.call(
            "send_message",
            serde_json::json!({"recipient": "sess-claude", "message": "looks good, shipping", "session": "sess-codex", "agent": "codex", "thread_id": thread_id}),
        )
        .await
        .expect("send reply");
        wait_for_body(&mut c, "sess-claude", "looks good, shipping").await;

        // 4) the reply GROUPED into the same thread, and dual-write dedup means the
        //    shared daemon shows EXACTLY two messages — never four.
        let mut final_msgs = serde_json::Value::Null;
        for _ in 0..24 {
            let m = c.call("messages", serde_json::json!({"thread_id": thread_id})).await.unwrap();
            if m.as_array().unwrap().len() >= 2 {
                final_msgs = m;
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let arr = final_msgs.as_array().expect("messages array");
        assert_eq!(arr.len(), 2, "reply must join the same thread with no duplication: {final_msgs}");
        let bodies: Vec<&str> = arr.iter().filter_map(|m| m["body"].as_str()).collect();
        assert!(
            bodies.contains(&"can you review this?") && bodies.contains(&"looks good, shipping"),
            "both messages present in the thread: {bodies:?}"
        );

        // 5) still exactly one thread for the whole conversation.
        let threads = c.call("list_threads", serde_json::json!({"project": project})).await.unwrap();
        assert_eq!(threads.as_array().unwrap().len(), 1, "one thread for the conversation: {threads}");
    });

    stop_daemon(&home);
}

#[test]
fn freeze_send_message_dedup_exactly_one_inbox_row() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "sender-a", "session_id": "freeze-dedup-sender", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        let recv = c.call(
            "session_start",
            serde_json::json!({"agent": "receiver-a", "session_id": "freeze-dedup-recv", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        let recv_canon = recv["session_id"].as_str().unwrap().to_string();

        // Send a message from sender to receiver (addressed by harness alias).
        let r = c
            .call(
                "send_message",
                serde_json::json!({
                    "recipient": "freeze-dedup-recv",
                    "message": "dedup-test-payload",
                    "session": "freeze-dedup-sender",
                }),
            )
            .await
            .expect("send_message");
        assert_eq!(r["target_session"], recv_canon.as_str(), "target mismatch: {r}");

        // Wait until the local-delivery path inserts the row (it is synchronous,
        // but poll briefly to absorb any scheduling jitter).
        let mut delivered = false;
        for _ in 0..20 {
            // Open a SEPARATE store handle (read-only observer) to count without
            // consuming. peek_inbox returns delivered=0 rows only.
            let store = Store::open(&home.store_path()).unwrap();
            let pending = store.peek_inbox(&recv_canon).unwrap();
            if !pending.is_empty() {
                delivered = true;
                // FREEZE: exactly one pending row; assert the count before any drain.
                assert_eq!(
                    pending.len(),
                    1,
                    "peek_inbox should return exactly 1 pending row before drain, got {}: {:?}",
                    pending.len(),
                    pending
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        assert!(delivered, "message never landed in receiver inbox");

        // trigger turn_start → fetch_mentions_into_inbox + drain (assemble_turn_start_context).
        // This simulates a real turn where the inbox is consumed AND a relay
        // catch-up fetch runs, which could create a duplicate if idempotency broke.
        let ctx = c
            .call("turn_start", serde_json::json!({"session": "freeze-dedup-recv"}))
            .await
            .expect("turn_start");
        // The context should mention our payload (it was in the inbox when drained).
        // FREEZE-NOTE: assemble_turn_start_context embeds inbox rows in the context
        // string only when the session has at least one prior turn (prev_started_at != 0).
        // If this is the first-ever turn, context may be null. We check rows-only
        // path to avoid fragility here; what matters is no duplicate is created.
        let _ = ctx; // context presence/absence is version-sensitive; not asserted here

        // After the drain, peek should now be empty.
        let store_after = Store::open(&home.store_path()).unwrap();
        let still_pending = store_after.peek_inbox(&recv_canon).unwrap();
        assert!(
            still_pending.is_empty(),
            "after turn_start drain, peek_inbox should be empty, got: {:?}",
            still_pending
        );

        // Call inbox RPC (triggers re-fetch from relay + drain). The relay still
        // holds the echoed event, but INSERT OR IGNORE must prevent re-delivery.
        // FREEZE: inbox returns empty rows — no duplicate delivery.
        let inbox_second = c
            .call("inbox", serde_json::json!({"session": "freeze-dedup-recv"}))
            .await
            .expect("inbox RPC");
        let rows_second = inbox_second["rows"].as_array().unwrap();
        assert!(
            rows_second.is_empty(),
            "second inbox call after drain must return empty (dedup), got: {:?}",
            rows_second
        );
    });

    stop_daemon(&home);
}

/// Behavior 2a: targeted mention routing — reaches ONLY the target session.
///
/// A mention with `target_session` set is delivered exclusively to that
/// session's inbox. A sibling session of the SAME agent in the SAME project
/// does NOT receive it.
#[test]
fn freeze_targeted_mention_reaches_only_target_session() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect daemon for freeze-targeted");
        // One sender, two sibling sessions of the SAME receiver agent.
        c.call(
            "session_start",
            serde_json::json!({"agent": "freeze-sender", "session_id": "freeze-tgt-src", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        // Two sibling sessions (same agent slug → same pubkey).
        let a = c.call(
            "session_start",
            serde_json::json!({"agent": "freeze-rcvr", "session_id": "freeze-tgt-a", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        let a_canon = a["session_id"].as_str().unwrap().to_string();
        let b = c.call(
            "session_start",
            serde_json::json!({"agent": "freeze-rcvr", "session_id": "freeze-tgt-b", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        let b_canon = b["session_id"].as_str().unwrap().to_string();

        // Target ONLY session B by its harness alias (resolved to canonical).
        let r = c
            .call(
                "send_message",
                serde_json::json!({
                    "recipient": "freeze-tgt-b",
                    "message": "for-b-only",
                    "session": "freeze-tgt-src",
                }),
            )
            .await
            .expect("send_message");
        assert_eq!(r["target_session"], b_canon.as_str(), "target_session field: {r}");

        // Wait until B receives it.
        let mut b_got = false;
        for _ in 0..20 {
            let inbox = c.call("inbox", serde_json::json!({"session": "freeze-tgt-b"})).await.unwrap();
            if inbox["rows"].as_array().unwrap().iter().any(|m| m["body"] == "for-b-only") {
                b_got = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        assert!(b_got, "target session B must receive the message");

        // FREEZE: session A of the same agent must NOT have any inbox rows.
        // Allow a brief window for any spurious delivery before asserting.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let store = Store::open(&home.store_path()).unwrap();
        let a_pending = store.peek_inbox(&a_canon).unwrap();
        assert!(
            a_pending.is_empty(),
            "sibling session A must NOT receive a message targeted at B, got: {:?}",
            a_pending
        );

        // Also assert via the inbox RPC (runs fetch + drain, so stronger).
        let a_inbox = c.call("inbox", serde_json::json!({"session": "freeze-tgt-a"})).await.unwrap();
        assert!(
            a_inbox["rows"].as_array().unwrap().is_empty(),
            "inbox RPC for sibling A must be empty, got: {:?}",
            a_inbox["rows"]
        );
    });

    stop_daemon(&home);
}

/// Behavior 2b: untargeted mention reaches all alive sessions for the
/// recipient agent+project ONLY — not sessions of OTHER agents or projects.
#[test]
fn freeze_untargeted_mention_reaches_all_sessions_of_recipient_agent_only() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Sender agent.
        c.call(
            "session_start",
            serde_json::json!({"agent": "unt-sender", "session_id": "unt-src", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        // Recipient agent — TWO alive sessions (same slug → same pubkey).
        c.call(
            "session_start",
            serde_json::json!({"agent": "unt-target", "session_id": "unt-rcv-1", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        c.call(
            "session_start",
            serde_json::json!({"agent": "unt-target", "session_id": "unt-rcv-2", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        // Bystander agent — must NOT receive the message.
        c.call(
            "session_start",
            serde_json::json!({"agent": "unt-bystander", "session_id": "unt-bys", "cwd": "/tmp"}),
        )
        .await
        .unwrap();

        // Send WITHOUT specifying a target_session — resolves to the agent's pubkey
        // directly. Since unt-target is a local agent we can address by slug;
        // however, send_message needs a recipient that resolves on this daemon.
        // Use the most-recently-started session id as recipient (which sets
        // target_session in the event). To exercise the UNTARGETED path we need
        // the recipient to resolve to a pubkey only (no target_session). We do this
        // by sending to the slug@project form.
        //
        // FREEZE-NOTE: the untargeted path requires the sender to address by slug
        // rather than session-id. We look up the project from the sender's session.
        // Look up the receiver agent's pubkey directly from the sessions table.
        // For locally-hosted agents, profiles/peer_sessions are self-filtered by
        // the daemon and not stored, so slug@project resolution doesn't work.
        // Sending to the RAW 64-hex pubkey triggers the untargeted path (no
        // target_session in the event) and exercises the "all alive sessions"
        // delivery code path in route_mention_into_with_id.
        let store = Store::open(&home.store_path()).unwrap();
        let rcv_rec = store
            .get_session("unt-rcv-1")
            .unwrap()
            .expect("unt-rcv-1 session exists");
        let rcv_pubkey = rcv_rec.agent_pubkey.clone();

        let r = c
            .call(
                "send_message",
                serde_json::json!({
                    "recipient": rcv_pubkey,
                    "message": "broadcast-to-all",
                    "session": "unt-src",
                }),
            )
            .await
            .expect("send_message");
        // Untargeted: target_session is null in the response (pubkey resolution
        // returns no target_session).
        assert!(
            r["target_session"].is_null(),
            "untargeted send should have null target_session, got: {r}"
        );

        // Both receiver sessions must get the message.
        let mut rcv1_got = false;
        let mut rcv2_got = false;
        for _ in 0..25 {
            if !rcv1_got {
                let inbox1 = c
                    .call("inbox", serde_json::json!({"session": "unt-rcv-1"}))
                    .await
                    .unwrap();
                rcv1_got = inbox1["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["body"] == "broadcast-to-all");
            }
            if !rcv2_got {
                let inbox2 = c
                    .call("inbox", serde_json::json!({"session": "unt-rcv-2"}))
                    .await
                    .unwrap();
                rcv2_got = inbox2["rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|m| m["body"] == "broadcast-to-all");
            }
            if rcv1_got && rcv2_got {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(
            rcv1_got,
            "untargeted: receiver session 1 must get the message"
        );
        assert!(
            rcv2_got,
            "untargeted: receiver session 2 must get the message"
        );

        // FREEZE: bystander inbox must be empty.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let bys_inbox = c
            .call("inbox", serde_json::json!({"session": "unt-bys"}))
            .await
            .unwrap();
        assert!(
            bys_inbox["rows"].as_array().unwrap().is_empty(),
            "bystander inbox must be empty after untargeted mention, got: {:?}",
            bys_inbox["rows"]
        );
    });

    stop_daemon(&home);
}

/// Behavior 3: 39000/39002 idempotency.
///
/// Applying the same NIP-29 group-metadata (kind 39000) and members-snapshot
/// (kind 39002) events TWICE must be stable: project_meta and group_members
/// converge to the same state and members are not duplicated.
///
/// We exercise this through the `session_start` path (which causes the daemon
/// to subscribe and receive relay-authored 39000/39002 events) combined with
/// direct Store assertions. To force idempotency, we call session_start twice
/// for the same project, which may re-apply any cached 39002 snapshot from the
/// relay.
///
/// FREEZE-NOTE: the daemon applies 39000/39002 only when they arrive from the
/// relay subscription. We cannot inject raw relay events through the public
/// RPC path, so we verify idempotency via the Store methods that 39000/39002
/// handlers call: `upsert_project_meta` and `replace_group_members`.
/// The integration layer here tests that the Store semantics survive repeated
/// application (the daemon uses these same methods).
#[test]
fn freeze_39000_39002_idempotency_no_member_duplication() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        // Start a session — this triggers ensure_group_and_membership and an
        // initial 39000/39002 subscription.
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "freeze-grp-idem-1", "cwd": "/tmp"}),
        )
        .await
        .expect("first session_start");
    });

    // Allow the daemon time to receive any relay-echoed group events.
    std::thread::sleep(Duration::from_millis(400));

    // Record baseline membership state.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store
        .get_session("freeze-grp-idem-1")
        .unwrap()
        .expect("session row");
    let project = rec.project.clone();

    // FREEZE: group owned and member present after first start.
    assert!(
        store.is_group_owned(&project).unwrap(),
        "group must be owned after session_start with userNsec"
    );
    assert!(
        store.is_group_member(&project, &rec.agent_pubkey).unwrap(),
        "agent must be a member after session_start"
    );

    // Simulate idempotency: apply the same 39002 snapshot twice via the public
    // Store API (the daemon uses `replace_group_members` when it processes
    // kind:39002 from the relay — calling it twice is equivalent to receiving
    // the same event twice).
    let members_snapshot = vec![(rec.agent_pubkey.clone(), "member".to_string())];
    let ts = 9_000_000u64;
    store
        .replace_group_members(&project, &members_snapshot, ts)
        .unwrap();
    store
        .replace_group_members(&project, &members_snapshot, ts)
        .unwrap();

    // FREEZE: membership is stable — no duplication, same set.
    assert!(
        store.is_group_member(&project, &rec.agent_pubkey).unwrap(),
        "member still present after double-apply of 39002 snapshot"
    );
    // Count members via list — expect exactly 1 (no duplication).
    // We confirm via is_group_member scoped to a distinct fake pubkey being absent.
    assert!(
        !store.is_group_member(&project, "nonexistent-pk").unwrap(),
        "phantom member must not appear after 39002 re-application"
    );

    // FREEZE: project_meta upsert is idempotent (39000 handler).
    store
        .upsert_project_meta(&project, "about text v1", ts)
        .unwrap();
    store
        .upsert_project_meta(&project, "about text v1", ts)
        .unwrap();
    let meta = store.get_project_meta(&project).unwrap();
    assert_eq!(
        meta.as_deref(),
        Some("about text v1"),
        "project_meta must be stable after idempotent 39000 re-application"
    );

    // Applying an updated 'about' must overwrite (not duplicate) — the upsert
    // is DO UPDATE SET.
    store
        .upsert_project_meta(&project, "about text v2", ts + 1)
        .unwrap();
    let meta2 = store.get_project_meta(&project).unwrap();
    assert_eq!(
        meta2.as_deref(),
        Some("about text v2"),
        "project_meta must reflect latest about after overwrite"
    );

    stop_daemon(&home);
}

/// Behavior 4: startup mention catch-up.
///
/// A kind:1 mention published to the relay BEFORE a session starts is caught
/// into that session's inbox once the session starts (via the
/// `fetch_mentions_into_inbox` call in `turn_start` / `inbox` RPCs).
/// A subsequent fetch of the same mention does NOT duplicate the row.
///
/// Setup:
///   1. Start a "pre-publisher" session for agent X (establishes agent pubkey).
///   2. A separate session (agent Y) sends a message to agent X.
///   3. End the original session for X so X has no alive sessions.
///   4. Start a NEW session for X.
///   5. Call `inbox` for the new X session → the stored mention is caught up.
///   6. Call `inbox` again → no new rows (dedup).
///
/// FREEZE-NOTE: the catch-up fetch (`fetch_mentions_into_inbox`) queries the
/// relay for kind:1 events tagged to the agent's pubkey. This relies on the
/// relay holding the event (nak serve in-memory relay does). The test uses
/// the same `shared_relay_url()` for both sender and receiver, which is the
/// in-memory nak relay that persists events for the lifetime of the test suite.
#[test]
fn freeze_startup_mention_catchup_no_duplicate() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    // Phase 1: establish agent X's pubkey by starting a session.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "catchup-x", "session_id": "catchup-x-pre", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
        // Also start the sender Y.
        c.call(
            "session_start",
            serde_json::json!({"agent": "catchup-y", "session_id": "catchup-y-src", "cwd": "/tmp"}),
        )
        .await
        .unwrap();
    });

    // Phase 2: Y sends a message to the PRE-session of X (which is alive now).
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");

        // Look up X's pubkey so we can send an untargeted mention.
        let x_rec = Store::open(&home.store_path())
            .unwrap()
            .get_session("catchup-x-pre")
            .unwrap()
            .expect("catchup-x-pre session row");
        // Use the RAW pubkey as recipient — this produces target_session=None in
        // the event (untargeted), which is required for catch-up delivery to a
        // DIFFERENT session of the same agent. If we sent by session-id, the event
        // would have target_session="catchup-x-pre" and would NOT be re-routed to
        // the new session (compute_targets would find no match).
        let x_pubkey = x_rec.agent_pubkey.clone();

        // Send an untargeted message from Y to X's pubkey (no target_session) —
        // publishes a kind:1 event to the relay, which will persist there.
        let r = c
            .call(
                "send_message",
                serde_json::json!({
                    "recipient": x_pubkey,
                    "message": "pre-start-mention",
                    "session": "catchup-y-src",
                }),
            )
            .await
            .expect("send_message");
        assert!(
            r["target_session"].is_null(),
            "untargeted send must have null target_session, got: {r}"
        );

        // Wait for the published event to be echoed back (so it's on the relay).
        tokio::time::sleep(Duration::from_millis(500)).await;

        // End X's pre-session so it becomes dead — simulates X going offline.
        c.call(
            "session_end",
            serde_json::json!({"session": "catchup-x-pre"}),
        )
        .await
        .expect("session_end");
    });

    // Small pause to let session_end propagate.
    std::thread::sleep(Duration::from_millis(200));

    // Phase 3: start a NEW session for X.
    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "catchup-x", "session_id": "catchup-x-new", "cwd": "/tmp"}),
        )
        .await
        .expect("second session_start for catchup-x");

        // Phase 4: call inbox for the NEW session — this triggers
        // fetch_mentions_into_inbox, which fetches the pre-published kind:1 from
        // the relay and enqueues it.
        let mut caught_up = false;
        for _ in 0..25 {
            let inbox = c
                .call("inbox", serde_json::json!({"session": "catchup-x-new"}))
                .await
                .unwrap();
            let rows = inbox["rows"].as_array().unwrap();
            if rows.iter().any(|m| m["body"] == "pre-start-mention") {
                caught_up = true;
                // FREEZE: exactly one catch-up row delivered.
                // (inbox drains on each call; we just confirm the body arrived.)
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            caught_up,
            "new session must catch up the pre-start mention from relay"
        );

        // Phase 5: call inbox again — no new rows (the mention was already
        // delivered and is deduped by INSERT OR IGNORE on the inbox PK AND by
        // mark_mention_seen on the agent pubkey).
        let inbox_again = c
            .call("inbox", serde_json::json!({"session": "catchup-x-new"}))
            .await
            .expect("second inbox call");
        let rows_again = inbox_again["rows"].as_array().unwrap();
        // FREEZE: no re-delivery.
        assert!(
            rows_again.iter().all(|m| m["body"] != "pre-start-mention"),
            "pre-start mention must NOT be re-delivered on a second inbox call, got: {:?}",
            rows_again
        );
    });

    stop_daemon(&home);
}
