use super::*;

#[test]
fn agent_roster_materializes_per_h_tag() {
    let store = Store::open_memory().unwrap();
    let backend = Keys::generate();
    let backend_pk = backend.public_key().to_hex();
    store.upsert_channel("root-a", "Root A", "", "", 1).unwrap();
    store.upsert_channel("root-b", "Root B", "", "", 1).unwrap();
    store
        .replace_channel_admins("root-a", std::slice::from_ref(&backend_pk), 1)
        .unwrap();
    store
        .replace_channel_admins("root-b", std::slice::from_ref(&backend_pk), 1)
        .unwrap();
    let event = build_at(
        &backend,
        crate::fabric::nip29::wire::KIND_AGENT_ROSTER,
        "",
        vec![
            make_tag(&["d", "codex"]),
            make_tag(&["hostname", "laptop"]),
            make_tag(&["use-criteria", "For coding"]),
            make_tag(&["h", "root-a"]),
            make_tag(&["h", "root-b"]),
        ],
        123,
    );

    Nip29Materializer::materialize_agent_roster(&store, &event);

    let root_a = store.list_agent_roster_for_channel("root-a").unwrap();
    assert_eq!(root_a.len(), 1);
    assert_eq!(root_a[0].backend_pubkey, backend_pk);
    assert_eq!(root_a[0].slug, "codex");
    assert_eq!(root_a[0].host, "laptop");
    assert_eq!(root_a[0].use_criteria, "For coding");
    assert_eq!(root_a[0].updated_at, 123);
    assert_eq!(
        store.list_agent_roster_for_channel("root-b").unwrap().len(),
        1
    );
}

#[test]
fn agent_roster_rejects_non_admin_or_non_root_h_tags() {
    let store = Store::open_memory().unwrap();
    let backend = Keys::generate();
    let backend_pk = backend.public_key().to_hex();
    store.upsert_channel("root", "Root", "", "", 1).unwrap();
    store
        .upsert_channel("child", "Child", "", "root", 1)
        .unwrap();
    store
        .replace_channel_admins("child", std::slice::from_ref(&backend_pk), 1)
        .unwrap();

    let event = build_at(
        &backend,
        crate::fabric::nip29::wire::KIND_AGENT_ROSTER,
        "",
        vec![
            make_tag(&["d", "codex"]),
            make_tag(&["hostname", "laptop"]),
            make_tag(&["h", "root"]),
            make_tag(&["h", "child"]),
        ],
        123,
    );

    Nip29Materializer::materialize_agent_roster(&store, &event);

    assert!(store
        .list_agent_roster_for_channel("root")
        .unwrap()
        .is_empty());
    assert!(store
        .list_agent_roster_for_channel("child")
        .unwrap()
        .is_empty());
}
