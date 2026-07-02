use super::*;
use crate::state::{RegisterSession, Store};
use crate::util::CHAT_RENDER_WORD_LIMIT;

fn message(words: usize) -> String {
    (0..words)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn long_message_guard_requires_explicit_override() {
    let long = ChatWriteParams {
        message: message(CHAT_RENDER_WORD_LIMIT + 1),
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
        message: message(CHAT_RENDER_WORD_LIMIT),
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
fn mention_label_resolution_treats_nested_channels_under_same_root_as_same_project() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("root", "project", "", "", 1).unwrap();
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
    assert_eq!(resolved.project, "leaf-b");
}
