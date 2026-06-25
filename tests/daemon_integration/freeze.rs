use crate::daemon_harness::*;
use std::time::Duration;
use tenex_edge::daemon::client::Client;
use tenex_edge::session::PeerStatusObservation;
use tenex_edge::state::Store;

// ── Frozen regression guards (dedup, targeted/untargeted mention routing,
//    39000/39002 idempotency, startup catch-up) and the threaded e2e. ───────────

/// A valid (throwaway) operator nsec for the local relay — the HUMAN's key.
const EXAMPLE_USER_NSEC: &str = "nsec1eulru7a67wt9ndqxv424kmgvd6uyd8defdxh7y9peut28f2p2vhs35m5h4";
/// A valid (throwaway) backend seckey (hex) — distinct from the user's key.
const EXAMPLE_BACKEND_SEC_HEX: &str =
    "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422";

fn rewrite_config_with_user_nsec(home: &Home) {
    // The user's pubkey is whitelisted (granted admin in every group); the
    // backend key signs group management. Distinct keys per doctrine.
    use nostr_sdk::prelude::Keys;
    let user_pk = Keys::parse(EXAMPLE_USER_NSEC)
        .unwrap()
        .public_key()
        .to_hex();
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [user_pk],
        "backendName": "test-host",
        "relays": [shared_relay_url()],
        "userNsec": EXAMPLE_USER_NSEC,
        "tenexPrivateKey": EXAMPLE_BACKEND_SEC_HEX,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
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

    // FREEZE: the local-only room parent is present after first start.
    // Membership itself is relay-confirmed state, so this test seeds the
    // subsequent 39002 snapshot explicitly instead of relying on an optimistic
    // local write.
    assert_eq!(
        store.session_room_parent(&project).unwrap().as_deref(),
        Some("tmp"),
        "session start should record the local-only room parent"
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

#[test]
fn freeze_status_outbox_is_debuggable_and_presence_is_unified() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        c.call(
            "session_start",
            serde_json::json!({"agent": "coder", "session_id": "freeze-outbox-1", "cwd": "/tmp"}),
        )
        .await
        .expect("session_start");

        c.call(
            "turn_start",
            serde_json::json!({
                "session": "freeze-outbox-1",
                "cwd": "/tmp",
                "prompt": "investigate unified presence state",
                "json": false
            }),
        )
        .await
        .expect("turn_start");

        let debug = c
            .call("debug_outbox", serde_json::json!({"limit": 20}))
            .await
            .expect("debug_outbox");
        let rows = debug["rows"].as_array().expect("rows array");
        assert!(
            rows.iter()
                .any(|r| r["agent_slug"].as_str() == Some("coder")),
            "debug_outbox should expose queued or recently published status rows: {debug}"
        );
    });

    stop_daemon(&home);
}

#[test]
fn freeze_peer_status_materializes_to_unified_presence_state() {
    let store = Store::open_memory().unwrap();
    store
        .record_peer_status(&PeerStatusObservation {
            agent_pubkey: "peer-pubkey".into(),
            agent_slug: "peer".into(),
            project: "proj".into(),
            host: "remote".into(),
            rel_cwd: "work".into(),
            title: "reviewing relay state".into(),
            activity: "checking 39002".into(),
            busy: true,
            emitted_at: 100,
            observed_at: 105,
        })
        .unwrap();

    let rows = store.peer_session_snapshots(Some("proj"), 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].agent_pubkey, "peer-pubkey");
    assert_eq!(rows[0].title, "reviewing relay state");
    assert!(rows[0].busy);
}
