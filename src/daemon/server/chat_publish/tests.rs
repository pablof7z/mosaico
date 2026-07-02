use super::*;
use crate::state::{RegisterSession, Store};

fn register_session(store: &Store, external_id: &str) -> String {
    store
        .register_session(&RegisterSession {
            harness: "codex".into(),
            external_id_kind: "harness_session".into(),
            external_id: external_id.into(),
            agent_pubkey: "agent-pk".into(),
            agent_slug: "agent".into(),
            channel_h: "h1".into(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 1,
        })
        .unwrap()
}

fn inject(store: &Store, session_id: &str, event_id: &str, body: &str, delivered_at: u64) {
    store
        .enqueue_inbox(event_id, session_id, "human-pk", "h1", body, 10)
        .unwrap();
    let rows = store
        .claim_pending_for_session(session_id, delivered_at)
        .unwrap();
    let ids = rows.into_iter().map(|row| row.event_id).collect::<Vec<_>>();
    store.mark_injected_for_echo(&ids, session_id).unwrap();
}

#[test]
fn delayed_injected_prompt_echo_consumes_matching_event_group() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile("human-pk", "Pablo", "pablo", "local", false, 1)
        .unwrap();
    let sid = register_session(&store, "s1");
    inject(&store, &sid, "ev1", "@agent hello", 20);

    assert!(consume_injected_prompt_echo_in_store(
        &store,
        &sid,
        "<@pablo> @agent hello",
        &["human-pk".into()],
        3600,
    )
    .unwrap());
    assert!(store.injected_for_session(&sid).unwrap().is_empty());
    assert!(
        !consume_injected_prompt_echo_in_store(
            &store,
            &sid,
            "<@pablo> @agent hello",
            &["human-pk".into()],
            3601,
        )
        .unwrap(),
        "consumed echo records must not eat later human repeats"
    );
}

#[test]
fn same_injected_text_is_scoped_to_session() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile("human-pk", "Pablo", "pablo", "local", false, 1)
        .unwrap();
    let sid1 = register_session(&store, "s1");
    let sid2 = register_session(&store, "s2");
    inject(&store, &sid1, "ev1", "@agent hello", 20);
    inject(&store, &sid2, "ev2", "@agent hello", 20);

    assert!(consume_injected_prompt_echo_in_store(
        &store,
        &sid1,
        "<@pablo> @agent hello",
        &["human-pk".into()],
        30,
    )
    .unwrap());
    assert!(store.injected_for_session(&sid1).unwrap().is_empty());
    assert_eq!(store.injected_for_session(&sid2).unwrap().len(), 1);
}
