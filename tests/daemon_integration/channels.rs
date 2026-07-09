use crate::daemon_harness::*;
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "channels/create.rs"]
mod create;
#[path = "channels/edit.rs"]
mod edit;
#[path = "channels/human_who.rs"]
mod human_who;
#[path = "channels/launch_mentions.rs"]
mod launch_mentions;
#[path = "channels/native_context.rs"]
mod native_context;
#[path = "channels/orchestration.rs"]
mod orchestration;
#[path = "channels/pty_bootstrap.rs"]
mod pty_bootstrap;
#[path = "channels/session_kill.rs"]
mod session_kill;
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

fn rewrite_config_with_user_nsec_without_backend_key(home: &Home, per_session_rooms: bool) {
    write_config_with_backend_key(home, per_session_rooms, None);
}

fn write_config(home: &Home, per_session_rooms: bool) {
    write_config_with_backend_key(home, per_session_rooms, Some(EXAMPLE_BACKEND_SEC_HEX));
}

/// Write the daemon config, choosing whether human-initiated sessions mint a
/// per-session room (`per_session_rooms`).
fn write_config_with_backend_key(home: &Home, per_session_rooms: bool, backend_key: Option<&str>) {
    // NIP-29 ownership/minting needs a NIP-29-aware relay; nak can't do it.
    // The user's pubkey is whitelisted (so it's granted admin in every group),
    // and the backend key signs group management. The two keys are ALWAYS
    // distinct per doctrine: userNsec = human, tenexPrivateKey = backend.
    let user_pk = pubkey_of(EXAMPLE_USER_NSEC);
    let cfg = home.dir.path().join("config.json");
    let mut body = serde_json::json!({
        "whitelistedPubkeys": [user_pk],
        "backendName": "test-host",
        "relays": [shared_nip29_relay_url()],
        "indexerRelay": shared_nip29_relay_url(),
        "userNsec": EXAMPLE_USER_NSEC,
        "perSessionRooms": per_session_rooms,
    });
    if let Some(key) = backend_key {
        body["tenexPrivateKey"] = serde_json::Value::String(key.to_string());
    }
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
}

fn refresh_channel_members(channel: &str) {
    let _ = tenex_edge::daemon::blocking::call(
        "channel_members",
        serde_json::json!({ "channel": channel }),
    );
}

fn materialize_member_snapshot(home: &Home, channel: &str, pubkey: &str) {
    Store::open(&home.store_path())
        .unwrap()
        .replace_channel_members(channel, &[pubkey.to_string()], 9_000_000)
        .unwrap();
}

async fn precreate_channel_group_as_user(channel: &str) {
    use nostr_sdk::prelude::*;

    let relay = shared_nip29_relay_url();
    let user_keys = Keys::parse(EXAMPLE_USER_NSEC).unwrap();
    let client = Client::builder()
        .signer(user_keys.clone())
        .opts(ClientOptions::default().automatic_authentication(true))
        .build();
    client.add_relay(&relay).await.unwrap();
    client.connect().await;
    client
        .wait_for_connection(std::time::Duration::from_secs(8))
        .await;
    let _ = client
        .fetch_events(
            Filter::new().kind(Kind::from(0u16)).limit(1),
            std::time::Duration::from_secs(5),
        )
        .await;

    for (label, builder) in [
        (
            "9007 create-group",
            tenex_edge::fabric::nip29::lifecycle::group_create(channel).unwrap(),
        ),
        (
            "9002 lock-closed",
            tenex_edge::fabric::nip29::lifecycle::group_lock_closed(channel).unwrap(),
        ),
    ] {
        let signed = builder.sign_with_keys(&user_keys).unwrap();
        let out = client.send_event(&signed).await.unwrap();
        assert!(
            !out.success.is_empty(),
            "{label} should be accepted by the NIP-29 relay: {:?}",
            out.failed
        );
    }
}

fn unique_session(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{prefix}-{nanos}")
}
