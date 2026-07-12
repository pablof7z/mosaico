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
        repair_whitelisted_admins: true,
    }
}

#[test]
fn materialized_relay_cache_proves_existing_member_ready() {
    let (_dir, store) = ready_store("", &["admin"], &["member"]);

    assert!(store_locally_materialized_ready(
        &store,
        &context("member", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_relay_cache_does_not_prove_missing_member_ready() {
    let (_dir, store) = ready_store("", &["admin"], &["member"]);

    assert!(!store_locally_materialized_ready(
        &store,
        &context("other", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_relay_cache_does_not_prove_missing_admin_ready() {
    let (_dir, store) = ready_store("", &["other-admin"], &["member"]);

    assert!(!store_locally_materialized_ready(
        &store,
        &context("member", None),
        &["admin".to_string()]
    ));
}

#[test]
fn materialized_subgroup_needs_relay_parent_consent_check() {
    let (_dir, store) = ready_store("parent", &["admin"], &["member"]);

    assert!(!store_locally_materialized_ready(
        &store,
        &context("member", Some("parent")),
        &["admin".to_string()]
    ));
}
