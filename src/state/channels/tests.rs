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

#[cfg(test)]
mod group_hierarchy_tests {
    use super::*;
    use crate::observed_views::{TestGroup, TestGroupDelivery};

    fn hierarchy_store() -> Store {
        let store = Store::open_memory().unwrap();
        store.install_test_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root").metadata("general", "", "", 1),
            TestGroup::new("root/task").metadata("task", "", "root", 2),
            TestGroup::new("root/task/sub").metadata("sub", "", "root/task", 3),
        ]));
        store
    }

    #[test]
    fn rootness_and_subchannel_predicates_route_through_group_projection() {
        let store = hierarchy_store();
        assert!(store.is_root_channel("root").unwrap());
        assert!(!store.is_root_channel("root/task").unwrap());
        assert!(
            !store.is_root_channel("never").unwrap(),
            "unknown is not root"
        );
        assert!(store.is_subchannel("root/task").unwrap());
        assert!(store.is_subchannel("root/task/sub").unwrap());
        assert!(!store.is_subchannel("root").unwrap());
        assert!(!store.is_subchannel("never").unwrap());
    }

    #[test]
    fn root_channel_of_walks_parent_links_to_the_top() {
        let store = hierarchy_store();
        assert_eq!(
            store.root_channel_of("root/task/sub").unwrap(),
            Some("root".to_string())
        );
        assert_eq!(
            store.root_channel_of("root/task").unwrap(),
            Some("root".to_string())
        );
        assert_eq!(
            store.root_channel_of("root").unwrap(),
            Some("root".to_string())
        );
        assert_eq!(
            store.root_channel_of("never").unwrap(),
            None,
            "unknown ancestry is not root"
        );
    }

    #[test]
    fn parent_children_and_named_lookup_reflect_projection() {
        let store = hierarchy_store();
        assert_eq!(
            store.channel_parent("root/task").unwrap().as_deref(),
            Some("root")
        );
        assert_eq!(store.channel_parent("root").unwrap().as_deref(), Some(""));
        assert_eq!(store.channel_parent("never").unwrap(), None);

        let roots: Vec<_> = store
            .list_root_channels()
            .unwrap()
            .into_iter()
            .map(|c| c.channel_h)
            .collect();
        assert_eq!(roots, vec!["root"]);

        let children: Vec<_> = store
            .list_child_channels("root")
            .unwrap()
            .into_iter()
            .map(|c| c.channel_h)
            .collect();
        assert_eq!(children, vec!["root/task"]);

        assert_eq!(
            store
                .channel_id_for_name("root", "task")
                .unwrap()
                .as_deref(),
            Some("root/task")
        );
        assert!(store
            .channel_id_for_name("root", "missing")
            .unwrap()
            .is_none());

        let all: Vec<_> = store
            .list_channels()
            .unwrap()
            .into_iter()
            .map(|c| c.channel_h)
            .collect();
        assert_eq!(all.len(), 3);
        assert!(store.get_channel("root/task").unwrap().is_some());
        assert!(store.get_channel("never").unwrap().is_none());
    }
}
