use super::*;

#[test]
fn loading_ordinary_agent_rejects_redundant_keys_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    load_or_create(dir.path(), "coder", "codex", None, 1).unwrap();
    let path = dir.path().join("agents/coder.json");
    let keys = Keys::generate();
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    config["secret_key"] = serde_json::json!(keys.secret_key().to_secret_hex());
    config["public_key"] = serde_json::json!(keys.public_key().to_hex());
    let inconsistent = serde_json::to_string_pretty(&config).unwrap();
    std::fs::write(&path, &inconsistent).unwrap();

    let error = load(dir.path(), "coder").unwrap_err();
    assert!(
        format!("{error:#}").contains("perSessionKey:true forbids secret_key and public_key"),
        "unexpected error: {error:#}"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), inconsistent);
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
