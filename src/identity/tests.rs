use super::*;

mod byline;
mod commands;

#[test]
fn generates_then_reloads_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", 100).unwrap();
    let b = load_or_create(dir.path(), "coder", 200).unwrap();
    assert_eq!(a.pubkey_hex(), b.pubkey_hex());
    assert_eq!(
        a.keys.secret_key().to_secret_hex(),
        b.keys.secret_key().to_secret_hex()
    );
}

#[test]
fn distinct_slugs_get_distinct_keys() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", 1).unwrap();
    let b = load_or_create(dir.path(), "reviewer", 1).unwrap();
    assert_ne!(a.pubkey_hex(), b.pubkey_hex());
}

#[test]
fn rejects_bad_slug() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_or_create(dir.path(), "bad slug/with-stuff", 1).is_err());
    assert!(load_or_create(dir.path(), "", 1).is_err());
}

#[test]
fn persists_to_expected_path() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", 1).unwrap();
    assert!(dir.path().join("agents").join("coder.json").exists());
}

#[test]
fn add_local_agent_creates_then_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (a, created) = add_local_agent(dir.path(), "coder", None, 1).unwrap();
    assert!(created, "first add mints a fresh key");
    assert!(dir.path().join("agents").join("coder.json").exists());

    let (b, created2) = add_local_agent(dir.path(), "coder", None, 2).unwrap();
    assert!(!created2, "re-adding an existing slug does not recreate");
    assert_eq!(a.pubkey_hex(), b.pubkey_hex());
}

#[test]
fn remove_local_agent_parks_then_reports_missing() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", 1).unwrap();
    let live = dir.path().join("agents").join("coder.json");
    assert!(live.exists());

    let parked = remove_local_agent(dir.path(), "coder").unwrap();
    let parked = parked.expect("removing an existing agent returns the parked path");
    assert!(!live.exists(), "live key file is gone");
    assert!(parked.exists(), "key is parked, not unlinked");
    assert!(list_local_agent_details(dir.path()).is_empty());
    assert!(list_local_pubkeys(dir.path()).is_empty());

    assert!(remove_local_agent(dir.path(), "coder").unwrap().is_none());
}

fn mgmt_secret() -> SecretKey {
    SecretKey::from_slice(&[0x01u8; 32]).unwrap()
}

#[test]
fn session_key_v2_determinism() {
    let sk = mgmt_secret();
    let a = derive_session_keys_v2(&sk, "sess-abc");
    let b = derive_session_keys_v2(&sk, "sess-abc");
    assert_eq!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "derive_session_keys_v2 must be deterministic"
    );
    assert_eq!(
        a.secret_key().to_secret_hex(),
        b.secret_key().to_secret_hex()
    );
}

#[test]
fn session_key_v2_different_sessions_differ() {
    let sk = mgmt_secret();
    let a = derive_session_keys_v2(&sk, "session-1");
    let b = derive_session_keys_v2(&sk, "session-2");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "different session ids must yield different session pubkeys"
    );
}

#[test]
fn session_key_v2_cross_machine_divergence() {
    // The management secret is per-machine: the same session id on two machines
    // (two management keys) must yield two different keypairs.
    let machine_a = mgmt_secret();
    let machine_b = SecretKey::from_slice(&[0x02u8; 32]).unwrap();
    let a = derive_session_keys_v2(&machine_a, "same-session-id");
    let b = derive_session_keys_v2(&machine_b, "same-session-id");
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "cross-machine: distinct management keys must diverge for the same session id"
    );
}

#[test]
fn ac1_resumed_session_derives_same_pubkey() {
    let sk = mgmt_secret();
    let session_id = "claude-native-xKz8-resume-test";

    let original = derive_session_keys_v2(&sk, session_id);
    let resumed = derive_session_keys_v2(&sk, session_id);

    assert_eq!(
        original.public_key().to_hex(),
        resumed.public_key().to_hex(),
        "AC1: a resumed session must reproduce the exact same session pubkey"
    );
    assert_eq!(
        original.secret_key().to_secret_hex(),
        resumed.secret_key().to_secret_hex(),
        "AC1: and the exact same secret key"
    );
}

// ── SessionIdentity ───────────────────────────────────────────────────────────

#[test]
fn session_identity_agent_ref_names_pubkey_by_codename() {
    let inst = SessionIdentity::new(
        "deadbeef".into(),
        "claude".into(),
        "sess-123".into(),
        "willow-echo-042".into(),
    );
    assert_eq!(inst.display_slug(), "claude/willow-echo-042");
    let aref = inst.agent_ref();
    assert_eq!(aref.pubkey, "deadbeef");
    assert_eq!(aref.slug, "claude/willow-echo-042");
}

#[test]
fn session_identity_fallback_codename_is_short_code_of_session() {
    let inst = SessionIdentity::fallback("sess-xyz", "claude".into(), "deadbeef".into());
    assert_eq!(inst.pubkey, "deadbeef");
    assert_eq!(inst.slug, "claude");
    assert_eq!(inst.session_id, "sess-xyz");
    assert_eq!(inst.codename, crate::util::friendly_short_code("sess-xyz"));
    assert_eq!(
        inst.display_slug(),
        format!("claude/{}", crate::util::friendly_short_code("sess-xyz"))
    );
}
