use super::*;
use nostr_sdk::ToBech32 as _;

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
        "relays": [crate::config::DEFAULT_RELAY],
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
fn local_relay_requires_and_normalizes_an_owner() {
    let key = nostr_sdk::Keys::generate()
        .public_key()
        .to_bech32()
        .unwrap();
    let mut doc = baseline_document();
    let mut options = opts();
    options.local_relay = true;
    options.operator_pubkeys = Some(key);

    apply_overrides(&mut doc, &options).unwrap();
    ensure_complete(&mut doc).unwrap();
    let setup = summarize(&doc, &options).unwrap();

    assert!(setup.local_relay);
    assert!(setup.start_local_relay);
    assert_eq!(doc["relays"], json!([LOCAL_RELAY_URL]));
    assert_eq!(setup.owner_pubkey.unwrap().len(), 64);
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
