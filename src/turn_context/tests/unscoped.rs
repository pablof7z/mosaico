use super::SELF_PK;
use crate::state::{RegisterSession, Store};
use std::sync::Mutex;

#[test]
fn first_turn_explains_unscoped_state_without_fake_channel_warnings() {
    let store = Mutex::new(Store::open_memory().unwrap());
    let session = {
        let store = store.lock().unwrap();
        store
            .reserve_hook_session_for_test(&RegisterSession {
                pubkey: SELF_PK.into(),
                observed_harness: "codex".into(),
                agent_slug: "test-agent".into(),
                launch_channel_h: String::new(),
                work_root: String::new(),
                child_pid: None,
                now: 100,
            })
            .unwrap();
        store.get_session(SELF_PK).unwrap().unwrap()
    };

    let context =
        super::super::render_turn_start_text_for_test(&store, &session, "", "", 0).unwrap();
    assert!(
        context.contains("has not joined any Mosaico channels"),
        "{context}"
    );
    assert!(context.contains("normal filesystem access"), "{context}");
    assert!(!context.contains("not a member"), "{context}");
    assert!(!context.contains("<workspace name=\"\""), "{context}");
}
