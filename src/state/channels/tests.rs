use super::*;

#[test]
fn root_channels_read_model_sorted_by_id() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("task", "Task", "not a channel", "beta", 3)
        .unwrap();
    store.upsert_channel("beta", "Beta", "two", "", 2).unwrap();
    store
        .upsert_channel("alpha", "Alpha", "one", "", 1)
        .unwrap();

    let channels = store.list_root_channels().unwrap();
    let ids = channels
        .iter()
        .map(|channel| channel.channel_h.as_str())
        .collect::<Vec<_>>();

    assert_eq!(ids, vec!["alpha", "beta"]);
    assert_eq!(channels[0].human_name(), Some("Alpha"));
    assert_eq!(channels[0].about, "one");
}

#[test]
fn channel_meta_read_model_returns_parent_metadata() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("task", "Task", "child", "channel", 1)
        .unwrap();

    let meta = store.channel_meta_read_model("task").unwrap().unwrap();
    assert_eq!(meta.parent, "channel");
    assert_eq!(meta.human_name(), Some("Task"));
}

#[test]
fn only_named_siblings_share_a_unique_namespace() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("root-a", "general", "", "", 1)
        .unwrap();
    store
        .upsert_channel("root-b", "general", "", "", 1)
        .unwrap();
    store
        .upsert_channel("unnamed-a", "", "", "root-a", 1)
        .unwrap();
    store
        .upsert_channel("unnamed-b", "", "", "root-a", 1)
        .unwrap();
    store
        .upsert_channel("named-a", "planning", "", "root-a", 1)
        .unwrap();

    assert!(store
        .upsert_channel("named-b", "planning", "", "root-a", 1)
        .is_err());
    store
        .upsert_channel("named-other", "planning", "", "root-b", 1)
        .unwrap();
}

#[test]
fn list_child_channels_and_purge_deleted_channel() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "Root", "r", "", 1).unwrap();
    store
        .upsert_channel("child", "Child", "c", "root", 2)
        .unwrap();
    store
        .upsert_channel_member("child", "pk", "member", 2)
        .unwrap();

    let children = store.list_child_channels("root").unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].channel_h, "child");

    store.purge_deleted_channel("child").unwrap();
    assert!(store.get_channel("child").unwrap().is_none());
    assert!(store.list_channel_members("child").unwrap().is_empty());
    assert!(store.list_child_channels("root").unwrap().is_empty());

    store
        .upsert_workspace("root", "/tmp/root-workspace", 3)
        .unwrap();
    store.purge_deleted_channel("root").unwrap();
    assert!(store.get_channel("root").unwrap().is_none());
    assert!(store.workspace_path("root").unwrap().is_none());
}

#[test]
fn archived_channel_predicate_uses_about_prefix() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_channel("active", "Active", "normal work", "channel", 1)
        .unwrap();
    store
        .upsert_channel("archived", "Archived", "[ARCHIVED] done", "channel", 1)
        .unwrap();

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
    assert!(!store.is_archived_channel("active").unwrap());
    assert!(store.is_archived_channel("archived").unwrap());
    assert!(!store.is_archived_channel("missing").unwrap());
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
fn managed_channel_proof_excludes_unrelated_observed_groups_and_survives_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        store.upsert_channel("root", "Root", "", "", 1).unwrap();
        store
            .upsert_workspace("root", "/tmp/managed-root", 1)
            .unwrap();
        store
            .reserve_channel_resolution_intent("root", "child", "child", 2)
            .unwrap();
        store
            .upsert_channel("child", "Child", "", "root", 3)
            .unwrap();
        store
            .upsert_channel("unrelated", "Elsewhere", "", "", 4)
            .unwrap();
    }

    let reopened = Store::open(&path).unwrap();
    assert!(reopened.is_managed_channel("root").unwrap());
    assert!(reopened.is_managed_channel("child").unwrap());
    assert!(!reopened.is_managed_channel("unrelated").unwrap());
    assert_eq!(
        reopened
            .list_managed_channels()
            .unwrap()
            .into_iter()
            .map(|channel| channel.channel_h)
            .collect::<Vec<_>>(),
        vec!["child", "root"]
    );
}
