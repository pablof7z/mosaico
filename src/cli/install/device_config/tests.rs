use super::*;

fn opts() -> InstallOpts {
    InstallOpts::default()
}

#[test]
fn overrides_preserve_unknown_fields_and_secrets() {
    let backend = crate::config::generate_mosaico_private_key();
    let operator = crate::config::generate_mosaico_private_key();
    let mut doc = json!({
        "unknown": "keep",
        "userNsec": operator.clone(),
        "mosaicoPrivateKey": backend.clone(),
        "relays": ["wss://relay.example"],
    });
    let mut options = opts();
    options.host_label = Some("workstation".into());
    options.per_session_rooms = Some(true);

    apply_overrides(&mut doc, &options).unwrap();
    ensure_complete(&mut doc).unwrap();

    assert_eq!(doc["unknown"], "keep");
    assert_eq!(doc["userNsec"], operator);
    assert_eq!(doc["mosaicoPrivateKey"], backend);
    assert_eq!(doc["backendName"], "workstation");
    assert_eq!(doc["perSessionRooms"], true);
}

#[test]
fn fresh_document_requires_an_explicit_relay_choice() {
    let mut doc = baseline_document();
    let error = ensure_complete(&mut doc).unwrap_err().to_string();

    assert!(error.contains("externally operated relay URL"));
}

#[test]
fn malformed_document_is_not_coerced() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("config.json");
    std::fs::write(&path, "[]").unwrap();

    let error = read_document(&path).unwrap_err().to_string();

    assert!(error.contains("must contain a JSON object"));
}

#[test]
fn relay_validation_rejects_non_websocket_urls() {
    let error = document::normalize_relay("https://relay.example")
        .unwrap_err()
        .to_string();
    assert!(error.contains("ws:// or wss://"));
}
