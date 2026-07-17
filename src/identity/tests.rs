use super::*;

mod byline;
mod record_validation;

#[test]
fn creates_then_reloads_keyless_agent_config() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", "yolo-claude", Some("reviewer"), 100).unwrap();
    let b = load_or_create(dir.path(), "coder", "ignored", None, 200).unwrap();
    assert!(a.keys.is_none());
    assert!(b.keys.is_none());
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("agents/coder.json")).unwrap(),
    )
    .unwrap();
    assert!(stored.get("secret_key").is_none());
    assert!(stored.get("public_key").is_none());
}

#[test]
fn distinct_ordinary_slugs_do_not_get_persisted_keys() {
    let dir = tempfile::tempdir().unwrap();
    let a = load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    let b = load_or_create(dir.path(), "reviewer", "claude", None, 1).unwrap();
    assert!(a.pubkey_hex().is_none());
    assert!(b.pubkey_hex().is_none());
}

#[test]
fn rejects_bad_slug() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_or_create(dir.path(), "bad slug/with-stuff", "codex", None, 1).is_err());
    assert!(load_or_create(dir.path(), "", "codex", None, 1).is_err());
}

#[test]
fn persists_to_expected_path() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    assert!(dir.path().join("agents").join("coder.json").exists());
}

#[test]
fn per_session_key_defaults_true_and_can_select_durable_mode() {
    let dir = tempfile::tempdir().unwrap();
    let default = load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    assert!(default.per_session_key);

    let path = dir.path().join("agents").join("coder.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let keys = Keys::generate();
    config["perSessionKey"] = serde_json::json!(false);
    config["secret_key"] = serde_json::json!(keys.secret_key().to_secret_hex());
    config["public_key"] = serde_json::json!(keys.public_key().to_hex());
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let durable = load_or_create(dir.path(), "coder", "ignored", None, 2).unwrap();
    assert!(!durable.per_session_key);
    assert!(default.pubkey_hex().is_none());
    assert_eq!(durable.pubkey_hex(), Some(keys.public_key().to_hex()));
}

#[test]
fn add_local_agent_creates_then_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let (a, created) = add_local_agent(dir.path(), "coder", "codex", Some("reviewer"), 1).unwrap();
    assert!(created, "first add creates the launch config");
    assert!(dir.path().join("agents").join("coder.json").exists());

    let (b, created2) = add_local_agent(dir.path(), "coder", "yolo-claude", None, 2).unwrap();
    assert!(!created2, "re-adding an existing slug does not recreate");
    assert!(a.pubkey_hex().is_none());
    assert!(b.pubkey_hex().is_none());
    assert_eq!(b.harness, "yolo-claude");
    assert_eq!(b.profile, None);
}

#[test]
fn remove_local_agent_permanently_unlinks_then_reports_missing() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    let live = dir.path().join("agents").join("coder.json");
    assert!(live.exists());

    assert!(remove_local_agent(dir.path(), "coder").unwrap());
    assert!(!live.exists(), "live key file is gone");
    assert!(!dir.path().join("agents/coder.json.removed").exists());
    assert!(keystore_entries(dir.path()).is_empty());
    assert!(list_local_pubkeys(dir.path()).is_empty());

    assert!(!remove_local_agent(dir.path(), "coder").unwrap());
}

#[test]
fn structured_save_transitions_identity_mode_and_preserves_owned_fields() {
    let dir = tempfile::tempdir().unwrap();
    add_local_agent(dir.path(), "coder", "codex-pty", Some("reviewer"), 10).unwrap();
    set_local_agent_byline(dir.path(), "coder", Some("  Reviews changes  ".into())).unwrap();

    let (durable, created) = save_local_agent(
        dir.path(),
        "coder",
        LocalAgentUpdate {
            harness: "codex-app".into(),
            profile: None,
            per_session_key: Some(false),
            byline: None,
        },
        99,
    )
    .unwrap();
    assert!(!created);
    assert!(!durable.per_session_key);
    let durable_pubkey = durable.pubkey_hex().expect("durable key generated");
    let path = dir.path().join("agents/coder.json");
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(stored["created_at"], 10);
    assert_eq!(stored["byline"], "Reviews changes");
    assert_eq!(stored["harness"], "codex-app");
    assert_eq!(stored["public_key"], durable_pubkey);
    let keys = Keys::parse(stored["secret_key"].as_str().unwrap()).unwrap();
    assert_eq!(keys.public_key().to_hex(), durable_pubkey);

    let (preserved, _) = save_local_agent(
        dir.path(),
        "coder",
        LocalAgentUpdate {
            harness: "codex-app".into(),
            profile: None,
            per_session_key: None,
            byline: Some(None),
        },
        100,
    )
    .unwrap();
    assert!(!preserved.per_session_key);
    assert_eq!(
        preserved.pubkey_hex().as_deref(),
        Some(durable_pubkey.as_str())
    );
    assert!(keystore_entries(dir.path())[0].byline.is_none());

    let (per_session, _) = save_local_agent(
        dir.path(),
        "coder",
        LocalAgentUpdate {
            harness: "codex-pty".into(),
            profile: Some("reviewer".into()),
            per_session_key: Some(true),
            byline: Some(Some("  New role  ".into())),
        },
        101,
    )
    .unwrap();
    assert!(per_session.per_session_key);
    assert!(per_session.keys.is_none());
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(stored["created_at"], 10);
    assert_eq!(stored["byline"], "New role");
    assert!(stored.get("secret_key").is_none());
    assert!(stored.get("public_key").is_none());
}

#[test]
fn structured_save_can_create_a_durable_agent_directly() {
    let dir = tempfile::tempdir().unwrap();
    let (agent, created) = save_local_agent(
        dir.path(),
        "chief",
        LocalAgentUpdate {
            harness: "claude-acp".into(),
            profile: None,
            per_session_key: Some(false),
            byline: Some(Some("Coordinates work".into())),
        },
        7,
    )
    .unwrap();

    assert!(created);
    assert!(!agent.per_session_key);
    assert!(agent.keys.is_some());
    assert_eq!(
        keystore_entries(dir.path())[0].byline.as_deref(),
        Some("Coordinates work")
    );
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
