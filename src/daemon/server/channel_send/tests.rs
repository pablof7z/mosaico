use super::*;
use crate::state::{
    Profile, RegisterSession, Status, Store, TestGroup, TestGroupDelivery, TestRelayDelivery,
};

#[path = "tests/attachment_coaching.rs"]
mod attachment_coaching;
#[path = "tests/unhosted_return_path.rs"]
mod unhosted_return_path;
fn register_session(store: &Store, pubkey: &str, agent_slug: &str, channel_h: &str) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.to_string(),
            observed_harness: "codex".to_string(),
            agent_slug: agent_slug.to_string(),
            launch_channel_h: channel_h.to_string(),
            work_root: channel_h.to_string(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
}

fn profile(pubkey: &str, name: &str, agent_slug: &str, host: &str) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: name.into(),
        slug: name.into(),
        agent_slug: agent_slug.into(),
        host: host.into(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn status(pubkey: &str, slug: &str, expiration: u64) -> Status {
    Status {
        pubkey: pubkey.into(),
        channel_h: "channel".into(),
        slug: slug.into(),
        title: String::new(),
        activity: String::new(),
        workspace: String::new(),
        branch: String::new(),
        state: crate::session_state::SessionState::Idle,
        state_since: 1,
        last_seen: 1,
        updated_at: 1,
        expiration,
    }
}

#[test]
fn mention_label_resolution_treats_nested_channels_under_same_root_as_same_root() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("root").metadata("channel", "", "", 1),
        TestGroup::new("task-a").metadata("Task A", "", "root", 2),
        TestGroup::new("leaf-a").metadata("Leaf A", "", "task-a", 3),
        TestGroup::new("task-b").metadata("Task B", "", "root", 4),
        TestGroup::new("leaf-b").metadata("Leaf B", "", "task-b", 5),
    ]));
    register_session(&store, "helper-pubkey", "helper", "leaf-b");
    let allocation = store.allocate_handle("helper-pubkey", "helper", 1).unwrap();

    let resolved = resolve_recipient(&store, "leaf-a", "local", &allocation.handle).unwrap();

    assert_eq!(resolved.pubkey, "helper-pubkey");
    assert_eq!(resolved.channel, "leaf-a");
}

#[test]
fn host_qualified_ordinal_mention_resolves_remote_profile() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([profile(
        "remote-pk",
        "developer1@remoteBackend",
        "",
        "remoteBackend",
    )]));

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
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([profile(
        "remote-pk",
        "developer1@remoteBackend",
        "",
        "remoteBackend",
    )]));

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
fn dashed_session_handle_resolves_current_profile_row() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([profile(
                "remote-pk",
                "willow-echo-042-codex",
                "codex",
                "remoteBackend",
            )])
            .statuses([status(
                "remote-pk",
                "willow-echo-042-codex",
                i64::MAX as u64,
            )]),
    );

    let resolved =
        resolve_recipient(&store, "channel", "localBackend", "willow-echo-042-codex").unwrap();

    assert_eq!(resolved.pubkey, "remote-pk");
    assert_eq!(resolved.channel, "channel");
}

#[test]
fn current_profile_name_resolves_without_a_status_gate() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles([profile(
        "remote-pk",
        "codex-willow-echo-042",
        "codex",
        "localBackend",
    )]));

    let resolved =
        resolve_recipient(&store, "channel", "localBackend", "codex-willow-echo-042").unwrap();

    assert_eq!(resolved.pubkey, "remote-pk");
    assert_eq!(resolved.channel, "channel");
}

#[test]
fn duplicate_reclaim_profiles_never_route_to_old_status_owner() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([
                profile("old-pk", "shared-codex", "codex", "remote"),
                profile("new-pk", "shared-codex", "codex", "remote"),
            ])
            .statuses([status("old-pk", "shared-codex", 1)]),
    );

    let error = match resolve_recipient(&store, "channel", "local", "shared-codex") {
        Ok(_) => panic!("duplicate profile projections must be ambiguous"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("ambiguous"));
}

#[test]
fn untyped_profile_with_status_is_not_a_session_handle() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([profile("human-pk", "shared-name", "", "remote")])
            .statuses([status("human-pk", "shared-name", 1)]),
    );

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

#[test]
fn authored_limit_precedes_attachment_processing_and_guides_to_files() {
    let unsafe_attachment = crate::attachment::Attachment {
        label: "../escape".into(),
        path: "ignored".into(),
    };
    let error = prepare_outbound_message(
        &"x".repeat(CHANNEL_MESSAGE_CHAR_LIMIT + 1),
        &[unsafe_attachment],
    )
    .unwrap_err()
    .to_string();

    assert!(error.contains("too long"), "{error}");
    assert!(error.contains("--attach FILE"), "{error}");
    assert!(error.contains("coordination-guide.md"), "{error}");
    assert!(validate_authored_message(&"x".repeat(CHANNEL_MESSAGE_CHAR_LIMIT)).is_ok());
}

#[test]
fn unknown_channel_send_rpc_field_is_rejected() {
    let error = match parse_params(&serde_json::json!({
        "message": "hello",
        "long_message": true,
    })) {
        Ok(_) => panic!("unknown channel_send field was accepted"),
        Err(error) => error.to_string(),
    };
    assert!(error.contains("channel_send received unknown field \"long_message\""));
}
