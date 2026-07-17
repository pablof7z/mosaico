use super::*;
use crate::state::{RegisterSession, Store};

fn register_session(store: &Store, pubkey: &str, agent_slug: &str, channel_h: &str) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.to_string(),
            observed_harness: "codex".to_string(),
            agent_slug: agent_slug.to_string(),
            channel_h: channel_h.to_string(),
            child_pid: None,
            transcript_path: None,
            now: 1,
        })
        .unwrap();
}

#[test]
fn mention_label_resolution_treats_nested_channels_under_same_root_as_same_root() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "channel", "", "", 1).unwrap();
    store
        .upsert_channel("task-a", "Task A", "", "root", 2)
        .unwrap();
    store
        .upsert_channel("leaf-a", "Leaf A", "", "task-a", 3)
        .unwrap();
    store
        .upsert_channel("task-b", "Task B", "", "root", 4)
        .unwrap();
    store
        .upsert_channel("leaf-b", "Leaf B", "", "task-b", 5)
        .unwrap();
    register_session(&store, "helper-pubkey", "helper", "leaf-b");
    let allocation = store.allocate_handle("helper-pubkey", "helper", 1).unwrap();

    let resolved = resolve_recipient(&store, "leaf-a", "local", &allocation.handle).unwrap();

    assert_eq!(resolved.pubkey, "helper-pubkey");
    assert_eq!(resolved.channel, "leaf-b");
}

#[test]
fn host_qualified_ordinal_mention_resolves_remote_profile() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile(
            "remote-pk",
            "developer1@remoteBackend",
            "developer1",
            "remoteBackend",
            false,
            1,
        )
        .unwrap();

    let resolved = resolve_recipient(
        &store,
        "channel",
        "localBackend",
        "developer1@remoteBackend",
    )
    .unwrap();

    assert_eq!(resolved.pubkey, "remote-pk");
    assert_eq!(resolved.channel, "channel");
}

#[test]
fn host_qualified_mention_tolerates_stale_qualified_slug_cache() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile(
            "remote-pk",
            "developer1@remoteBackend",
            "developer1@remoteBackend",
            "remoteBackend",
            false,
            1,
        )
        .unwrap();

    let resolved = resolve_recipient(
        &store,
        "channel",
        "localBackend",
        "developer1@remoteBackend",
    )
    .unwrap();

    assert_eq!(resolved.pubkey, "remote-pk");
}

#[test]
fn dashed_session_handle_resolves_live_session_and_validates_agent() {
    let store = Store::open_memory().unwrap();
    register_session(&store, "codex-pubkey", "codex", "channel");
    let allocation = store.allocate_handle("codex-pubkey", "codex", 1).unwrap();
    let handle = allocation.handle;

    let resolved = resolve_recipient(&store, "channel", "localBackend", &handle).unwrap();

    assert_eq!(resolved.pubkey, "codex-pubkey");
    assert_eq!(resolved.channel, "channel");

    let wrong = format!("{handle}-haiku");
    let err = match resolve_recipient(&store, "channel", "localBackend", &wrong) {
        Ok(_) => panic!("mismatched agent-session handle should not resolve"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("can't resolve recipient"));
}

#[test]
fn dashed_session_handle_resolves_profile_cache() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile_with_agent_slug(
            "remote-pk",
            "willow-echo-042-codex",
            "willow-echo-042-codex",
            "codex",
            "remoteBackend",
            false,
            1,
        )
        .unwrap();
    store
        .upsert_status(&crate::state::Status {
            pubkey: "remote-pk".into(),
            channel_h: "channel".into(),
            slug: "willow-echo-042-codex".into(),
            title: String::new(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            last_seen: 1,
            updated_at: 1,
            expiration: i64::MAX as u64,
        })
        .unwrap();

    let resolved =
        resolve_recipient(&store, "channel", "localBackend", "willow-echo-042-codex").unwrap();

    assert_eq!(resolved.pubkey, "remote-pk");
    assert_eq!(resolved.channel, "channel");
}

#[test]
fn stale_profile_name_without_live_status_does_not_resolve() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile_with_agent_slug(
            "remote-pk",
            "codex-willow-echo-042",
            "codex-willow-echo-042",
            "codex",
            "localBackend",
            false,
            1,
        )
        .unwrap();

    let err = match resolve_recipient(&store, "channel", "localBackend", "codex-willow-echo-042") {
        Ok(_) => panic!("stale profile names are not handle authority"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("can't resolve recipient"));
}

#[test]
fn duplicate_reclaim_profiles_never_route_to_old_status_owner() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile_with_agent_slug(
            "old-pk",
            "shared-codex",
            "shared-codex",
            "codex",
            "remote",
            false,
            1,
        )
        .unwrap();
    store
        .upsert_status(&crate::state::Status {
            pubkey: "old-pk".into(),
            channel_h: "channel".into(),
            slug: "shared-codex".into(),
            title: String::new(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            last_seen: 1,
            updated_at: 1,
            expiration: 1,
        })
        .unwrap();
    store
        .upsert_profile_with_agent_slug(
            "new-pk",
            "shared-codex",
            "shared-codex",
            "codex",
            "remote",
            false,
            2,
        )
        .unwrap();

    let error = match resolve_recipient(&store, "channel", "local", "shared-codex") {
        Ok(_) => panic!("duplicate profile projections must be ambiguous"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn untyped_profile_with_status_is_not_a_session_handle() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile("human-pk", "shared-name", "shared-name", "remote", false, 1)
        .unwrap();
    store
        .upsert_status(&crate::state::Status {
            pubkey: "human-pk".into(),
            channel_h: "channel".into(),
            slug: "shared-name".into(),
            title: String::new(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            last_seen: 1,
            updated_at: 1,
            expiration: 1,
        })
        .unwrap();

    let error = match resolve_recipient(&store, "channel", "local", "shared-name") {
        Ok(_) => panic!("untyped profiles are not session handles"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("can't resolve recipient"));
}

#[test]
fn local_chat_cache_scope_matches_signed_event_target() {
    assert_eq!(chat_publish_scope("sender-room", None, None), "sender-room");
    assert_eq!(
        chat_publish_scope("sender-room", Some("explicit-room"), Some("mentioned-room")),
        "explicit-room"
    );
    assert_eq!(
        chat_publish_scope("sender-room", None, Some("mentioned-room")),
        "mentioned-room"
    );
}
