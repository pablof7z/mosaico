use super::*;

#[test]
fn snapshot_replaces_agents_and_workspaces_atomically() {
    let store = Store::open_memory().unwrap();
    let backend = Keys::generate();
    let pubkey = backend.public_key().to_hex();
    let first = crate::domain::Profile::backend_named(pubkey.clone(), "macos", "macos", Vec::new())
        .with_agents(vec![
            ("claude".into(), "General implementation".into()),
            ("ios-tester".into(), "Black-box iOS testing".into()),
        ])
        .with_workspaces(vec!["mosaico".into(), "napplets".into()]);
    Nip29Materializer::materialize_profile(&store, &first, 10, &provenance("profile-10"));

    let replacement =
        crate::domain::Profile::backend_named(pubkey.clone(), "macos", "macos", Vec::new())
            .with_agents(vec![("claude".into(), "Updated role".into())])
            .with_workspaces(vec!["mosaico".into()]);
    Nip29Materializer::materialize_profile(&store, &replacement, 11, &provenance("profile-11"));

    let stale = crate::domain::Profile::backend_named(pubkey.clone(), "old", "old", Vec::new())
        .with_agents(vec![("stale".into(), "Must not return".into())])
        .with_workspaces(vec!["stale-workspace".into()]);
    Nip29Materializer::materialize_profile(&store, &stale, 9, &provenance("profile-9"));

    let cached = store.get_profile(&pubkey).unwrap().unwrap();
    assert_eq!(cached.host, "macos");
    assert_eq!(
        cached.agents,
        [("claude".to_string(), "Updated role".to_string())]
    );
    assert_eq!(cached.workspaces, ["mosaico"]);
}
