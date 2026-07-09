use super::*;
use crate::state::{RegisterSession, Store};
use crate::util::CHAT_WRITE_CHAR_LIMIT;

fn message(chars: usize) -> String {
    "a".repeat(chars)
}

#[test]
fn long_message_guard_requires_explicit_override() {
    let long = ChatWriteParams {
        message: message(CHAT_WRITE_CHAR_LIMIT + 1),
        long_message: false,
        ..Default::default()
    };
    assert!(long_message_requires_override(&long));

    let allowed = ChatWriteParams {
        long_message: true,
        ..long
    };
    assert!(!long_message_requires_override(&allowed));

    let short = ChatWriteParams {
        message: message(CHAT_WRITE_CHAR_LIMIT),
        long_message: false,
        ..Default::default()
    };
    assert!(!long_message_requires_override(&short));
}

fn register_session(store: &Store, session_id: &str, agent_slug: &str, channel_h: &str) {
    store
        .upsert_session_row(
            session_id,
            &RegisterSession {
                harness: "codex".to_string(),
                external_id_kind: "harness_session".to_string(),
                external_id: session_id.to_string(),
                agent_pubkey: format!("{agent_slug}-pubkey"),
                agent_slug: agent_slug.to_string(),
                channel_h: channel_h.to_string(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            },
        )
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
    register_session(&store, "helper-session", "helper", "leaf-b");

    let resolved = resolve_recipient(&store, "leaf-a", "local", "helper").unwrap();

    assert_eq!(resolved.target_session.as_deref(), Some("helper-session"));
    assert_eq!(resolved.channel, "leaf-b");
}

#[test]
fn mention_resolution_store_errors_are_visible() {
    let err = handle_mention_resolution_error(
        "helper",
        anyhow::Error::new(rusqlite::Error::InvalidQuery),
    )
    .unwrap_err();

    assert!(err
        .to_string()
        .contains("failed to resolve mention @helper"));
}

#[test]
fn mention_resolution_unknown_handles_remain_silent() {
    handle_mention_resolution_error("ghost", anyhow::anyhow!("can't resolve recipient")).unwrap();
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
    assert_eq!(resolved.target_session, None);
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
