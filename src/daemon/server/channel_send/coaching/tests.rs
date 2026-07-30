use super::*;
use crate::state::{RegisterSession, Status};

fn status(pubkey: &str, slug: &str, state: SessionState) -> Status {
    Status {
        pubkey: pubkey.into(),
        channel_h: "room".into(),
        slug: slug.into(),
        title: String::new(),
        activity: String::new(),
        workspace: "workspace".into(),
        branch: String::new(),
        state,
        state_since: 10,
        last_seen: 10,
        updated_at: 10,
        expiration: 100,
    }
}

fn remote_agent(store: &Store, pubkey: &str, handle: &str, state: SessionState) {
    store
        .upsert_profile_with_agent_slug(pubkey, handle, handle, "codex", "remote", false, 10)
        .unwrap();
    store.upsert_status(&status(pubkey, handle, state)).unwrap();
}

fn store_with_members() -> Store {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("room", "Room", "", "", 1).unwrap();
    remote_agent(&store, "drift-pk", "drift-codex", SessionState::Idle);
    remote_agent(&store, "drizzle-pk", "drizzle-codex", SessionState::Working);
    store
        .upsert_profile("human-pk", "Pablo", "pablo", "remote", false, 10)
        .unwrap();
    store
        .upsert_profile("backend-pk", "backend", "backend", "remote", true, 10)
        .unwrap();
    store
        .replace_channel_members(
            "room",
            &[
                "self-pk".into(),
                "drift-pk".into(),
                "drizzle-pk".into(),
                "human-pk".into(),
                "backend-pk".into(),
            ],
            10,
        )
        .unwrap();
    store
}

#[test]
fn ack_detector_is_deliberately_narrow() {
    for message in ["ACK", "Got it!", "thanks", "👍"] {
        assert!(ack_like(message).is_some(), "{message}");
    }
    for message in [
        "Got it; I will send the patch",
        "done with the patch",
        "ok to merge?",
    ] {
        assert!(ack_like(message).is_none(), "{message}");
    }
}

#[test]
fn unique_prefix_matches_an_idle_agent_and_excludes_non_agents() {
    let store = store_with_members();
    let notice = untagged_agent_prefix(&store, "Drift: hello", "room", "self-pk", "backend-pk", 20)
        .unwrap()
        .unwrap();

    assert_eq!(notice.code, "untagged_agent_prefix");
    assert_eq!(notice.matched_agent.as_deref(), Some("drift-codex"));
    assert_eq!(notice.matched_agent_state, Some("idle"));
    assert!(notice.summary.contains("won't tag anyone"));

    assert!(
        untagged_agent_prefix(&store, "Pablo: hello", "room", "self-pk", "backend-pk", 20)
            .unwrap()
            .is_none()
    );
}

#[test]
fn ambiguous_prefix_lists_candidates_without_guessing() {
    let store = store_with_members();
    let notice = untagged_agent_prefix(&store, "Dr: hello", "room", "self-pk", "backend-pk", 20)
        .unwrap()
        .unwrap();

    assert_eq!(notice.code, "untagged_agent_prefix_ambiguous");
    assert_eq!(
        notice.candidates,
        vec!["drift-codex".to_string(), "drizzle-codex".to_string()]
    );
    assert_eq!(notice.matched_agent, None);
}

#[test]
fn exact_handle_wins_over_longer_prefix_matches() {
    let store = store_with_members();
    remote_agent(&store, "short-pk", "dr", SessionState::Working);
    store
        .upsert_channel_member("room", "short-pk", "member", 11)
        .unwrap();
    let notice = untagged_agent_prefix(&store, "DR: hello", "room", "self-pk", "", 20)
        .unwrap()
        .unwrap();

    assert_eq!(notice.matched_agent.as_deref(), Some("dr"));
    assert_eq!(notice.matched_agent_state, None);
}

#[test]
fn only_a_single_leading_token_and_colon_is_considered() {
    let store = store_with_members();
    for message in [
        "drift-codex:hello",
        "drift codex: hello",
        "context then Drift: hello",
        "@drift: hello",
    ] {
        assert!(
            untagged_agent_prefix(&store, message, "room", "self-pk", "", 20)
                .unwrap()
                .is_none(),
            "{message}"
        );
    }
}

#[test]
fn local_self_is_not_a_candidate() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("room", "Room", "", "", 1).unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "self-pk".into(),
            observed_harness: "codex".into(),
            agent_slug: "drift".into(),
            launch_channel_h: "room".into(),
            work_root: "room".into(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
    store.allocate_handle("self-pk", "drift", 1).unwrap();
    store
        .replace_channel_members("room", &["self-pk".into()], 1)
        .unwrap();

    assert!(
        untagged_agent_prefix(&store, "Drift: hi", "room", "self-pk", "", 2)
            .unwrap()
            .is_none()
    );
}
