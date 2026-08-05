use super::*;

fn registry(dir: &tempfile::TempDir) -> ClientRegistry {
    ClientRegistry::open(dir.path().join("mcp-clients.json"))
}

fn approved() -> Vec<String> {
    vec!["https://chatgpt.com".to_string()]
}

fn register(registry: &ClientRegistry, uris: &[&str]) -> String {
    registry
        .register(&json!({ "redirect_uris": uris }), &approved())
        .expect("registration accepted")["client_id"]
        .as_str()
        .expect("client_id")
        .to_string()
}

#[test]
fn an_authorize_redirect_must_match_the_registration_exactly() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(&dir);
    let client_id = register(&registry, &["https://chatgpt.com/callback"]);

    assert!(registry
        .ensure_registered(&client_id, "https://chatgpt.com/callback")
        .is_ok());
    for near_miss in [
        "https://chatgpt.com/callback.evil",
        "https://chatgpt.com/callback/",
        "https://chatgpt.com/callback?x=1",
        "https://chatgpt.com/",
        "https://evil.example/steal",
    ] {
        assert!(
            registry.ensure_registered(&client_id, near_miss).is_err(),
            "{near_miss} must not match the registered callback"
        );
    }
}

#[test]
fn an_unregistered_client_id_authorizes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(&dir);
    let error = registry
        .ensure_registered("mcpc_never-seen", "http://127.0.0.1:9000/cb")
        .unwrap_err();
    assert!(error.to_string().contains("unknown client_id"));
}

#[test]
fn registration_refuses_an_origin_the_operator_never_approved() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(&dir);
    let error = registry
        .register(
            &json!({ "redirect_uris": ["https://evil.example/steal"] }),
            &approved(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("mcpRedirectOrigins"));
    assert!(
        registry.lock().is_empty(),
        "a refused registration must leave nothing behind to match against"
    );
}

#[test]
fn registration_requires_a_non_empty_redirect_uri_array() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(&dir);
    assert!(registry.register(&json!({}), &approved()).is_err());
    assert!(registry
        .register(&json!({ "redirect_uris": [] }), &approved())
        .is_err());
    assert!(registry
        .register(&json!({ "redirect_uris": [42] }), &approved())
        .is_err());
}

#[test]
fn a_registration_survives_the_server_process() {
    let dir = tempfile::tempdir().unwrap();
    let client_id = register(&registry(&dir), &["https://chatgpt.com/callback"]);
    // A second `open` stands in for the next MCP server start.
    assert!(registry(&dir)
        .ensure_registered(&client_id, "https://chatgpt.com/callback")
        .is_ok());
}

#[test]
fn re_registering_the_same_client_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let registry = registry(&dir);
    let first = register(&registry, &["http://127.0.0.1:7000/cb"]);
    let second = register(&registry, &["http://127.0.0.1:7000/cb"]);
    assert_eq!(first, second);
    assert_eq!(registry.lock().len(), 1);
}

#[test]
fn an_unreadable_registry_opens_empty_rather_than_admitting_anything() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-clients.json");
    std::fs::write(&path, "{ not json").unwrap();
    let registry = ClientRegistry::open(path);
    assert!(registry
        .ensure_registered("mcpc_anything", "http://127.0.0.1:9000/cb")
        .is_err());
}

#[test]
fn the_registry_is_bounded_and_drops_its_oldest_entry_first() {
    let mut clients = HashMap::new();
    for index in 0..MAX_REGISTERED_CLIENTS + 3 {
        clients.insert(
            format!("mcpc_{index}"),
            RegisteredClient {
                redirect_uris: vec![format!("http://127.0.0.1:{}/cb", 9000 + index)],
                registered_at: 1_000 + index as u64,
            },
        );
    }
    evict_oldest_beyond_cap(&mut clients);
    assert_eq!(clients.len(), MAX_REGISTERED_CLIENTS);
    assert!(!clients.contains_key("mcpc_0"));
    assert!(clients.contains_key(&format!("mcpc_{}", MAX_REGISTERED_CLIENTS + 2)));
}
