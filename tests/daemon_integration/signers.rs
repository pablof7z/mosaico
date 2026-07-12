use crate::daemon_harness::*;
use std::time::{SystemTime, UNIX_EPOCH};
use tenex_edge::daemon::client::Client;
use tenex_edge::state::Store;

#[path = "signers/durable_agent.rs"]
mod durable_agent;
#[path = "signers/relay.rs"]
mod relay;

const EXAMPLE_USER_NSEC: &str = "nsec1eulru7a67wt9ndqxv424kmgvd6uyd8defdxh7y9peut28f2p2vhs35m5h4";
const EXAMPLE_BACKEND_SEC_HEX: &str =
    "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422";

fn pubkey_of(sec: &str) -> String {
    use nostr_sdk::prelude::Keys;
    Keys::parse(sec).unwrap().public_key().to_hex()
}

fn rewrite_config_with_signing_relay(home: &Home) {
    let _ = rewrite_config_with_nak_relay(home);
}

fn rewrite_config_with_nak_relay(home: &Home) -> String {
    let relay = shared_nip29_relay_url();
    let user_pk = pubkey_of(EXAMPLE_USER_NSEC);
    let cfg = home.dir.path().join("config.json");
    let body = serde_json::json!({
        "whitelistedPubkeys": [user_pk],
        "backendName": "test-host",
        "relays": [relay],
        "indexerRelay": relay,
        "userNsec": EXAMPLE_USER_NSEC,
        "tenexPrivateKey": EXAMPLE_BACKEND_SEC_HEX,
    });
    std::fs::write(&cfg, serde_json::to_string(&body).unwrap()).unwrap();
    relay
}

fn unique_channel(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("issue22-{label}-{nanos}")
}

async fn start_session(
    client: &mut Client,
    agent: &str,
    harness_id: Option<&str>,
    resume_id: Option<&str>,
    channel: &str,
) -> String {
    let mut params = serde_json::json!({
        "agent": agent,
        "cwd": "/tmp",
        "channel": channel,
        "harness": "codex",
    });
    if let Some(id) = harness_id {
        params["session_id"] = serde_json::json!(id);
    }
    if let Some(id) = resume_id {
        params["resume_id"] = serde_json::json!(id);
    }
    let v = client
        .call("session_start", params)
        .await
        .expect("session_start");
    v["session_id"].as_str().unwrap().to_string()
}

#[test]
fn duplicate_same_agent_same_channel_gets_transient_signer() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_signing_relay(&home);
    let channel = unique_channel("collision");
    let other_channel = unique_channel("other");

    let (first_id, second_id, other_id) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let first = start_session(&mut c, "claude", Some("issue22-first"), None, &channel).await;
        let second = start_session(&mut c, "claude", Some("issue22-second"), None, &channel).await;
        let other = start_session(
            &mut c,
            "claude",
            Some("issue22-other"),
            None,
            &other_channel,
        )
        .await;
        (first, second, other)
    });

    let store = Store::open(&home.store_path()).unwrap();
    let first_pubkey =
        session_identity_pubkey(&store, &first_id).expect("first session should mint a pubkey");
    let second_pubkey = session_identity_pubkey(&store, &second_id)
        .expect("second same-agent/channel session should mint a pubkey");
    assert_ne!(
        second_pubkey, first_pubkey,
        "each session mints its own key, so same-agent sessions in one channel differ"
    );
    let other_pubkey = session_identity_pubkey(&store, &other_id)
        .expect("same agent in a different channel should mint a pubkey");
    assert_ne!(
        other_pubkey, first_pubkey,
        "a distinct session (even same agent, different channel) mints a distinct pubkey"
    );

    stop_daemon(&home);
}

/// Issue #98 regression: two concurrent same-agent sessions in one channel must
/// publish DISTINCT, INTERNALLY CONSISTENT identities. The bug this guards: the
/// second instance published its kind:0 under the first pubkey AND labelled it
/// "claude2", clobbering the first instance's "claude1" profile — so both pubkeys
/// (or the wrong pubkey) ended up named "claude1". Here we prove each selected
/// pubkey carries its OWN label on the wire (kind:0) and through the local
/// instance identity that backs `who`.
#[test]
fn concurrent_same_agent_sessions_publish_consistent_identities() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    let relay = rewrite_config_with_nak_relay(&home);
    let channel = unique_channel("issue98-consistency");

    let (first_id, second_id) = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        let first = start_session(&mut c, "claude", Some("issue98-first"), None, &channel).await;
        let second = start_session(&mut c, "claude", Some("issue98-second"), None, &channel).await;
        (first, second)
    });

    let store = Store::open(&home.store_path()).unwrap();
    let first_pubkey = store
        .get_session(&first_id)
        .unwrap()
        .expect("first session")
        .agent_pubkey;
    let second_pubkey = session_identity_pubkey(&store, &second_id)
        .expect("second concurrent session should get a distinct ordinal pubkey");
    assert_ne!(
        first_pubkey, second_pubkey,
        "the two concurrent instances must select distinct pubkeys"
    );

    // Each session reports its OWN (pubkey, codename-agent) pair through the
    // identity that backs `who`.
    let first_instance = store
        .session_identity_for_session(&first_id)
        .unwrap()
        .expect("first session identity");
    let second_instance = store
        .session_identity_for_session(&second_id)
        .unwrap()
        .expect("second session identity");
    let first_handle = first_instance.display_slug();
    let second_handle = second_instance.display_slug();
    assert_eq!(first_instance.pubkey, first_pubkey);
    assert_eq!(first_instance.display_slug(), first_handle);
    assert_eq!(second_instance.pubkey, second_pubkey);
    assert_eq!(second_instance.display_slug(), second_handle);

    // kind:0 on the relay: each pubkey is named for ITS OWN handle, never
    // clobbering another session's profile.
    assert!(
        wait_until(std::time::Duration::from_secs(20), || {
            relay::kind0_name_for_author(&relay, &first_pubkey).as_deref() == Some(&first_handle)
                && relay::kind0_name_for_author(&relay, &second_pubkey).as_deref()
                    == Some(&second_handle)
        }),
        "kind:0 names must be self-consistent: first={:?} second={:?}",
        relay::kind0_name_for_author(&relay, &first_pubkey),
        relay::kind0_name_for_author(&relay, &second_pubkey),
    );

    stop_daemon(&home);
}

#[test]
fn duplicate_resume_reassert_preserves_selected_pubkey() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();
    rewrite_config_with_signing_relay(&home);
    let channel = unique_channel("resume");

    let duplicate_id = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        start_session(&mut c, "claude", Some("issue22-anchor-a"), None, &channel).await;
        start_session(
            &mut c,
            "claude",
            None,
            Some("issue22-resume-token"),
            &channel,
        )
        .await
    });
    let before = session_identity_pubkey(&Store::open(&home.store_path()).unwrap(), &duplicate_id)
        .expect("duplicate session should have selected pubkey");
    let instance = Store::open(&home.store_path())
        .unwrap()
        .session_identity_for_session(&duplicate_id)
        .unwrap()
        .expect("duplicate session identity");
    assert_eq!(
        instance.pubkey, before,
        "session identity should expose the minted pubkey"
    );

    let duplicate_id_after = rt().block_on(async {
        let mut c = Client::connect_or_spawn().await.expect("connect");
        start_session(
            &mut c,
            "claude",
            None,
            Some("issue22-resume-token"),
            &channel,
        )
        .await
    });

    let store = Store::open(&home.store_path()).unwrap();
    let after = session_identity_pubkey(&store, &duplicate_id_after)
        .expect("reasserted duplicate should keep selected pubkey");
    assert_eq!(
        duplicate_id, duplicate_id_after,
        "resume_id should resolve to the same canonical session"
    );
    assert_eq!(before, after, "resume must preserve selected pubkey");

    stop_daemon(&home);
}
