use super::*;

#[test]
fn loading_pre_keyless_agent_migrates_redundant_keys_atomically() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    let path = dir.path().join("agents/coder.json");
    let keys = Keys::generate();
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["secret_key"] = serde_json::json!(keys.secret_key().to_secret_hex());
    config["public_key"] = serde_json::json!(keys.public_key().to_hex());
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let loaded = load(dir.path(), "coder").unwrap();
    assert!(loaded.per_session_key);
    assert!(loaded.keys.is_none());
    let migrated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(migrated.get("secret_key").is_none());
    assert!(migrated.get("public_key").is_none());
}

#[test]
fn loading_agent_requires_explicit_identity_mode() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    let path = dir.path().join("agents/coder.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config.as_object_mut().unwrap().remove("perSessionKey");
    std::fs::write(&path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let error = load(dir.path(), "coder").unwrap_err();
    assert!(
        format!("{error:#}").contains("missing field `perSessionKey`"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn missing_durable_key_material_can_be_created_without_changing_agent_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agents/chief-of-staff.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{
  "slug": "chief-of-staff",
  "created_at": 7,
  "byline": "Coordinates work",
  "perSessionKey": false,
  "harness": "codex-pty",
  "profile": "chief-of-staff"
}"#,
    )
    .unwrap();

    assert_eq!(
        durable_key_status(dir.path(), "chief-of-staff").unwrap(),
        DurableKeyStatus::Missing
    );
    assert!(create_missing_durable_key(dir.path(), "chief-of-staff").unwrap());
    assert_eq!(
        durable_key_status(dir.path(), "chief-of-staff").unwrap(),
        DurableKeyStatus::Ready
    );

    let loaded = load(dir.path(), "chief-of-staff").unwrap();
    assert!(!loaded.per_session_key);
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(stored["created_at"], 7);
    assert_eq!(stored["byline"], "Coordinates work");
    assert_eq!(stored["harness"], "codex-pty");
    assert_eq!(stored["profile"], "chief-of-staff");
    let keys = Keys::parse(stored["secret_key"].as_str().unwrap()).unwrap();
    assert_eq!(stored["public_key"], keys.public_key().to_hex());
}

#[test]
fn completing_public_key_preserves_an_existing_durable_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agents/writer.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let keys = Keys::generate();
    let secret = keys.secret_key().to_secret_hex();
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::json!({
            "slug": "writer",
            "secret_key": secret,
            "created_at": 9,
            "perSessionKey": false,
            "harness": "codex-pty"
        }))
        .unwrap(),
    )
    .unwrap();

    assert_eq!(
        durable_key_status(dir.path(), "writer").unwrap(),
        DurableKeyStatus::Missing
    );
    assert!(create_missing_durable_key(dir.path(), "writer").unwrap());
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(stored["secret_key"], secret);
    assert_eq!(stored["public_key"], keys.public_key().to_hex());
}

#[test]
fn malformed_existing_secret_is_not_overwritten_by_repair() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agents/writer.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        r#"{
  "slug": "writer",
  "secret_key": "not-a-key",
  "created_at": 9,
  "perSessionKey": false,
  "harness": "codex-pty"
}"#,
    )
    .unwrap();
    let before = std::fs::read(&path).unwrap();

    assert!(durable_key_status(dir.path(), "writer").is_err());
    assert!(create_missing_durable_key(dir.path(), "writer").is_err());
    assert_eq!(std::fs::read(path).unwrap(), before);
}
