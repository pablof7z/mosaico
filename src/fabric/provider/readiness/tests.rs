use super::*;

fn ready_store(
    parent: &str,
    admins: &[&str],
    members: &[&str],
) -> (tempfile::TempDir, crate::state::Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = crate::state::Store::open(&dir.path().join("state.db")).unwrap();
    store
        .upsert_channel("room", "room", "", parent, 100)
        .unwrap();
    store
        .replace_channel_admins(
            "room",
            &admins.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            101,
        )
        .unwrap();
    store
        .replace_channel_members(
            "room",
            &members.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            102,
        )
        .unwrap();
    (dir, store)
}

fn context<'a>(expect_member: &'a str, parent_hint: Option<&'a str>) -> ChannelCtx<'a> {
    ChannelCtx {
        channel: "room",
        expect_member,
        parent_hint,
        name: None,
    }
}

#[test]
fn materialized_relay_cache_proves_existing_member_ready() {
    let (_dir, store) = ready_store("", &["admin"], &["member"]);

    assert!(local::store_ready(
        &store,
        &context("member", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_relay_cache_does_not_prove_missing_member_ready() {
    let (_dir, store) = ready_store("", &["admin"], &["member"]);

    assert!(!local::store_ready(
        &store,
        &context("other", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_relay_cache_does_not_prove_missing_admin_ready() {
    let (_dir, store) = ready_store("", &["other-admin"], &["member"]);

    assert!(!local::store_ready(
        &store,
        &context("member", None),
        &["admin".to_string()]
    ));
}

#[test]
fn managed_relay_cache_is_not_ready_while_an_obsolete_admin_remains() {
    let (_dir, store) = ready_store("", &["admin", "obsolete"], &["member"]);
    store
        .upsert_workspace("room", "/tmp/managed-room", 103)
        .unwrap();

    assert!(!local::store_ready(
        &store,
        &context("member", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_subgroup_needs_relay_parent_consent_check() {
    let (_dir, store) = ready_store("parent", &["admin"], &["member"]);

    assert!(!local::store_ready(
        &store,
        &context("member", Some("parent")),
        &["admin".to_string()]
    ));
}

#[test]
fn cold_nested_resolution_intents_preserve_every_ancestor() {
    let store = crate::state::Store::open_memory().unwrap();
    store
        .reserve_channel_resolution_intent("root", "middle", "middle-h", 1)
        .unwrap();
    store
        .reserve_channel_resolution_intent("middle-h", "leaf", "leaf-h", 2)
        .unwrap();

    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "leaf-h", None).unwrap(),
        Some("middle-h".into())
    );
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "middle-h", None).unwrap(),
        Some("root".into())
    );
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "root", None).unwrap(),
        None
    );

    store
        .upsert_channel("middle-h", "middle", "", "", 3)
        .unwrap();
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "middle-h", None).unwrap(),
        None,
        "relay-authored root metadata must suppress a stale pending ancestor"
    );
}

#[test]
fn execution_time_relay_metadata_overrides_captured_parent_hint() {
    let store = crate::state::Store::open_memory().unwrap();
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "room", Some("captured-parent")).unwrap(),
        Some("captured-parent".into())
    );

    store.upsert_channel("room", "room", "", "", 1).unwrap();
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "room", Some("captured-parent")).unwrap(),
        None,
        "relay root truth arriving before execution must suppress the captured hint"
    );

    store
        .upsert_channel("room", "room", "", "relay-parent", 2)
        .unwrap();
    assert_eq!(
        ancestry::resolved_parent_hint_from_store(&store, "room", Some("captured-parent")).unwrap(),
        Some("relay-parent".into())
    );
}

#[tokio::test]
async fn managed_admin_removal_returns_the_exact_nmp_failure_without_local_roster_patch() {
    let configured = nostr::Keys::generate().public_key().to_hex();
    let removed = nostr::Keys::generate().public_key().to_hex();
    let state =
        crate::daemon::server::DaemonState::new_for_test_with_whitelisted(vec![configured.clone()])
            .await;
    let management = state.fabric_provider().management_pubkey().unwrap();
    state
        .with_store(|store| {
            store.upsert_channel("room", "Room", "", "", 1)?;
            store.upsert_workspace("room", "/tmp/managed-room", 1)?;
            store.replace_channel_admins("room", &[management, configured, removed.clone()], 2)?;
            store.replace_channel_members("room", &[], 2)
        })
        .unwrap();
    state
        .nmp()
        .script_write_error("terminal receipt", "relay explicitly rejected removal");

    let error = state
        .fabric_provider()
        .reconcile_managed_admins("room", None)
        .await
        .unwrap_err();
    let rendered = format!("{:#}", anyhow::Error::new(error));
    assert!(rendered.contains("one obsolete admin removal for 1 users"));
    assert!(rendered.contains("relay explicitly rejected removal"));
    assert!(state.with_store(|store| store.is_channel_admin("room", &removed).unwrap()));
}
