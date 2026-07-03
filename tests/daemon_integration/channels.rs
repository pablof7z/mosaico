use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "channels/chat_mirroring.rs"]
mod chat_mirroring;
#[path = "channels/create.rs"]
mod create;
#[path = "channels/edit.rs"]
mod edit;
#[path = "channels/launch_mentions.rs"]
mod launch_mentions;
#[path = "channels/native_context.rs"]
mod native_context;
#[path = "channels/orchestration.rs"]
mod orchestration;
#[path = "channels/replies.rs"]
mod replies;
#[path = "channels/session_lifecycle.rs"]
mod session_lifecycle;
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
