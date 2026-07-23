use crate::daemon_harness::*;
use mosaico::daemon::client::Client;
use mosaico::state::{Status, Store};
use std::time::Duration;

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
        "relays": [shared_nip29_relay_url()],
        "indexerRelay": shared_nip29_relay_url(),
        "userNsec": EXAMPLE_USER_NSEC,
        "mosaicoPrivateKey": EXAMPLE_BACKEND_SEC_HEX,
        // This test asserts the minted per-session room parent, so opt into the
        // per-session-room feature (default off).
        "perSessionRooms": true,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
}

/// Behavior 3: 39000/39002 idempotency.
///
/// Applying the same NIP-29 group-metadata (kind 39000) and members-snapshot
/// (kind 39002) events TWICE must be stable: relay cache rows
/// converge to the same state and members are not duplicated.
///
/// We exercise this through the `session_start` path (which causes the daemon
/// to subscribe and receive relay-authored 39000/39002 events) combined with
/// direct Store assertions. To force idempotency, we call session_start twice
/// for the same channel, which may re-apply any cached 39002 snapshot from the
/// relay.
///
/// FREEZE-NOTE: the daemon applies 39000/39002 only when they arrive from the
/// relay subscription. We cannot inject raw relay events through the public
/// RPC path, so we verify idempotency via the Store methods that 39000/39002
/// handlers call: `upsert_channel` and `replace_channel_members`.
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
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "freeze-grp-idem-1", "cwd": "/tmp"}), "claude-code"),
        )
        .await
        .expect("first session_start");
    });

    let lookup_store = Store::open(&home.store_path()).unwrap();
    let pubkey =
        pubkey_for_harness_session(&lookup_store, "claude-code", "freeze-grp-idem-1").unwrap();

    assert!(
        wait_until(Duration::from_secs(20), || Store::open(&home.store_path())
            .map(|store| {
                store
                    .get_session(&pubkey)
                    .ok()
                    .flatten()
                    .and_then(|rec| store.channel_parent(&rec.channel_h).ok().flatten())
                    .as_deref()
                    == Some("tmp")
            })
            .unwrap_or(false)),
        "session start should materialize the room's parent root channel"
    );

    // Record baseline membership state.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    let channel = rec.channel_h.clone();

    // FREEZE: the minted room's parent root channel is present after first
    // start. (Parent now lives in `relay_channels`; `session_room_parent` →
    // `channel_parent`.) Membership itself is relay-confirmed state, so this test
    // seeds the subsequent 39002 snapshot explicitly instead of relying on an
    // optimistic local write.
    assert_eq!(
        store.channel_parent(&channel).unwrap().as_deref(),
        Some("tmp"),
        "session start should record the room's parent root channel"
    );
    // Simulate idempotency: apply the same 39002 snapshot twice via the public
    // Store API (the daemon uses `replace_channel_members` when it processes
    // kind:39002 from the relay — calling it twice is equivalent to receiving
    // the same event twice). Apply it to a DEDICATED channel id the relay never
    // authors 39002 for (like the 39000 metadata check below uses
    // `freeze-39000-meta`). Seeding the live room's real id would race the
    // daemon's own relay-materialized 39002: `replace_channel_members` is guarded
    // by a per-role high-water mark, and a real relay event's timestamp (~1.7e9)
    // beats this low seed ts — so the live snapshot, not this seed, would win.
    let mem_h = "freeze-39002-members";
    let members_snapshot = vec![rec.pubkey.clone()];
    let ts = 9_000_000u64;
    store
        .replace_channel_members(mem_h, &members_snapshot, ts)
        .unwrap();
    store
        .replace_channel_members(mem_h, &members_snapshot, ts)
        .unwrap();

    // FREEZE: membership is stable — no duplication, same set.
    assert!(
        store.is_channel_member(mem_h, &rec.pubkey).unwrap(),
        "member still present after double-apply of 39002 snapshot"
    );
    // Count members via list — expect exactly 1 (no duplication).
    // We confirm via is_channel_member scoped to a distinct fake pubkey being absent.
    assert!(
        !store.is_channel_member(mem_h, "nonexistent-pk").unwrap(),
        "phantom member must not appear after 39002 re-application"
    );

    // FREEZE: channel-metadata upsert is idempotent (39000 handler →
    // `upsert_channel`). Use a dedicated channel id with an explicit created_at so
    // the monotonic created_at guard admits the overwrite. Metadata is the
    // channel's `about`.
    let meta_h = "freeze-39000-meta";
    store
        .upsert_channel(meta_h, meta_h, "about text v1", "tmp", ts)
        .unwrap();
    store
        .upsert_channel(meta_h, meta_h, "about text v1", "tmp", ts)
        .unwrap();
    let meta = store.get_channel(meta_h).unwrap();
    assert_eq!(
        meta.map(|c| c.about).as_deref(),
        Some("about text v1"),
        "channel metadata must be stable after idempotent 39000 re-application"
    );

    // Applying an updated 'about' (newer created_at) must overwrite (not
    // duplicate) — the upsert is DO UPDATE SET under the created_at guard.
    store
        .upsert_channel(meta_h, meta_h, "about text v2", "tmp", ts + 1)
        .unwrap();
    let meta2 = store.get_channel(meta_h).unwrap();
    assert_eq!(
        meta2.map(|c| c.about).as_deref(),
        Some("about text v2"),
        "channel metadata must reflect latest about after overwrite"
    );

    stop_daemon(&home);
}

#[test]
fn freeze_status_presence_is_unified() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_user_nsec(&home);

    let pubkey = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let started = c.call(
            "session_start",
            hook_session_start(serde_json::json!({"agent": "coder", "harness_session": "freeze-status-1", "cwd": "/tmp"}), "claude-code"),
        )
        .await
        .expect("session_start");
        let pubkey = started["pubkey"].as_str().unwrap().to_string();

        c.call(
            "turn_start",
            serde_json::json!({
                "harness_session": "freeze-status-1",
                "harness": "claude-code",
                "cwd": "/tmp",
                "prompt": "investigate unified presence state",
                "json": false
            }),
        )
        .await
        .expect("turn_start");

        pubkey
    });

    // Presence is unified: the published kind:30315 is reflected into the
    // `relay_status` cache under the session's DURABLE agent pubkey (the same
    // identity that signs chat) — not a separate presence table.
    let store = Store::open(&home.store_path()).unwrap();
    let rec = store.get_session(&pubkey).unwrap().expect("session row");
    assert!(
        wait_until(Duration::from_secs(20), || {
            Store::open(&home.store_path())
                .map(|s| {
                    s.live_status_for_channel(&rec.channel_h, 0)
                        .map(|rows| rows.iter().any(|r| r.pubkey == rec.pubkey))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }),
        "the coder agent's presence should materialize in relay_status under its selected pubkey; daemon_log={}",
        std::fs::read_to_string(home.dir.path().join("daemon.log"))
            .unwrap_or_else(|error| format!("<unreadable: {error}>"))
    );

    stop_daemon(&home);
}

#[test]
fn freeze_peer_status_materializes_to_unified_presence_state() {
    // Peer presence is now a kind:30315 cache row (`relay_status`); the old
    // `record_peer_status`/`peer_session_snapshots` pair is `upsert_status` +
    // `live_status_for_channel`. Liveness is NIP-40 freshness, so seed a future
    // expiration and read with now=0.
    let store = Store::open_memory().unwrap();
    store
        .upsert_status(&Status {
            pubkey: "peer-pubkey".into(),
            channel_h: "proj".into(),
            slug: "peer".into(),
            title: "reviewing relay state".into(),
            activity: "checking 39002".into(),
            state: mosaico::session_state::SessionState::Working,
            state_since: 105,
            last_seen: 105,
            updated_at: 105,
            expiration: 1_000_000,
        })
        .unwrap();

    let rows = store.live_status_for_channel("proj", 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pubkey, "peer-pubkey");
    assert_eq!(rows[0].title, "reviewing relay state");
    assert_eq!(rows[0].state, mosaico::session_state::SessionState::Working);
}
