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
}
