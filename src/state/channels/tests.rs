use super::*;

#[test]
fn archived_channel_predicate_uses_about_prefix() {
    assert!(is_archived_channel_about("[ARCHIVED] done"));
    assert!(!is_archived_channel_about("done [ARCHIVED]"));
    assert_eq!(archived_channel_about(""), "[ARCHIVED]");
    assert_eq!(archived_channel_about("done"), "[ARCHIVED] done");
    assert_eq!(archived_channel_about("[ARCHIVED] done"), "[ARCHIVED] done");
    assert_eq!(
        archived_channel_about(&"a".repeat(CHANNEL_ABOUT_MAX_CHARS))
            .chars()
            .count(),
        CHANNEL_ABOUT_MAX_CHARS
    );
}

#[test]
fn channel_resolution_intent_reuses_reserved_id_for_name() {
    let store = Store::open_memory().unwrap();

    let first = store
        .reserve_channel_resolution_intent("channel", "planning", "a1b2c3d4", 10)
        .unwrap();
    let second = store
        .reserve_channel_resolution_intent("channel", "planning", "ffffeeee", 11)
        .unwrap();

    assert_eq!(first, "a1b2c3d4");
    assert_eq!(second, first);
    assert_eq!(
        store
            .channel_resolution_intent("channel", "planning")
            .unwrap()
            .as_deref(),
        Some("a1b2c3d4")
    );
    assert_eq!(
        store
            .channel_resolution_parent("a1b2c3d4")
            .unwrap()
            .as_deref(),
        Some("channel")
    );
}

#[test]
fn purge_deleted_channel_removes_only_host_local_bindings() {
    let store = Store::open_memory().unwrap();
    store
        .reserve_channel_resolution_intent("root", "task", "child", 1)
        .unwrap();
    store.upsert_workspace("child", "/work/child", 1).unwrap();
    assert!(store.is_managed_channel("child").unwrap());

    store.purge_deleted_channel("child").unwrap();

    assert!(store.workspace_path("child").unwrap().is_none());
    assert!(store.channel_resolution_parent("child").unwrap().is_none());
    assert!(!store.is_managed_channel("child").unwrap());
}

#[test]
fn managed_channel_proof_survives_restart_without_relay_group_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .upsert_workspace("root", "/tmp/managed-root", 1)
            .unwrap();
        store
            .reserve_channel_resolution_intent("root", "child", "child", 2)
            .unwrap();
    }

    let reopened = Store::open(&path).unwrap();
    assert!(reopened.is_managed_channel("root").unwrap());
    assert!(reopened.is_managed_channel("child").unwrap());
    assert!(!reopened.is_managed_channel("unrelated").unwrap());
}
