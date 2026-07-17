use super::super::*;
use super::reg;

#[test]
fn identity_projection_uses_exact_handle_lease() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_derived_identity("agent", Some("willow"), 1, |_| {
            Ok(((), "derived".to_string()))
        })
        .unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "derived".into(),
            ..reg("codex", "ignored", "h1")
        })
        .unwrap();
    let identity = store.session_identity("derived").unwrap().unwrap();
    assert_eq!(identity.pubkey, "derived");
    assert_eq!(identity.handle, "willow-agent");
    assert!(!identity.durable_agent);
}

#[test]
fn configured_identity_uses_bare_slug_as_handle() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "configured".into(),
            ..reg("codex", "ignored", "h1")
        })
        .unwrap();
    let identity = store.session_identity("configured").unwrap().unwrap();
    assert_eq!(identity.handle, "agent");
    assert!(identity.durable_agent);
}

#[test]
fn workspace_paths_roundtrip() {
    let store = Store::open_memory().unwrap();
    store.upsert_workspace("h1", "/abs/path", 1).unwrap();
    assert_eq!(store.workspace_path("h1").unwrap().unwrap(), "/abs/path");
    store.upsert_workspace("h1", "/abs/other", 2).unwrap();
    assert_eq!(store.workspace_path("h1").unwrap().unwrap(), "/abs/other");
}
