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
fn per_session_key_defaults_true_and_can_select_durable_mode() {
    let dir = tempfile::tempdir().unwrap();
    let default = load_or_create(dir.path(), "coder", 1).unwrap();
    assert!(default.per_session_key);

    let path = dir.path().join("agents").join("coder.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["perSessionKey"] = serde_json::json!(false);
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let durable = load_or_create(dir.path(), "coder", 2).unwrap();
    assert!(!durable.per_session_key);
    assert_eq!(default.pubkey_hex(), durable.pubkey_hex());
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
fn signer_salt_reconstructs_the_same_session_key() {
    let sk = mgmt_secret();
    let salt = new_session_signer_salt();
    let a = derive_session_keys(&sk, &salt).unwrap();
    let b = derive_session_keys(&sk, &salt).unwrap();
    assert_eq!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "the persisted signer salt must reconstruct the signer"
    );
    assert_eq!(
        a.secret_key().to_secret_hex(),
        b.secret_key().to_secret_hex()
    );
}

#[test]
fn distinct_signer_salts_produce_distinct_session_keys() {
    let sk = mgmt_secret();
    let a = derive_session_keys(&sk, &new_session_signer_salt()).unwrap();
    let b = derive_session_keys(&sk, &new_session_signer_salt()).unwrap();
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "fresh salts must yield distinct session pubkeys"
    );
}

#[test]
fn signer_salt_cross_machine_divergence() {
    let machine_a = mgmt_secret();
    let machine_b = SecretKey::from_slice(&[0x02u8; 32]).unwrap();
    let salt = new_session_signer_salt();
    let a = derive_session_keys(&machine_a, &salt).unwrap();
    let b = derive_session_keys(&machine_b, &salt).unwrap();
    assert_ne!(
        a.public_key().to_hex(),
        b.public_key().to_hex(),
        "cross-machine: distinct management keys must diverge for the same session id"
    );
}

#[test]
fn malformed_signer_salt_is_rejected() {
    let sk = mgmt_secret();
    assert!(derive_session_keys(&sk, "not-a-signer-salt").is_err());
}

// ── SessionIdentity ───────────────────────────────────────────────────────────

#[test]
fn session_identity_agent_ref_names_pubkey_by_codename() {
    let inst = SessionIdentity::new(
        "deadbeef".into(),
        "claude".into(),
        "willow-echo-042-claude".into(),
        false,
    );
    assert_eq!(inst.display_slug(), "willow-echo-042-claude");
    let aref = inst.agent_ref();
    assert_eq!(aref.pubkey, "deadbeef");
    assert_eq!(aref.slug, "willow-echo-042-claude");
}

#[test]
fn durable_agent_identity_uses_bare_agent_slug() {
    let keys = Keys::generate();
    let identity =
        SessionIdentity::durable_agent(keys.public_key().to_hex(), "chief-of-staff".into());
    assert_eq!(identity.display_slug(), "chief-of-staff");
    assert!(identity.durable_agent);
}
