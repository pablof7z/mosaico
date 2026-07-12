use super::*;

#[test]
fn one_live_session_is_enforced_and_sequential_sessions_reuse_identity() {
    let store = Store::open_memory().unwrap();
    store
        .claim_durable_agent_session("pk", "chief", "session-a", 1)
        .unwrap();
    store
        .claim_durable_agent_session("pk", "chief", "session-a", 2)
        .unwrap();
    let error = store
        .claim_durable_agent_session("pk", "chief", "session-b", 3)
        .unwrap_err();
    assert!(error.to_string().contains("already has a live session"));

    store.release_durable_agent_session("session-a").unwrap();
    store
        .claim_durable_agent_session("pk", "chief", "session-b", 4)
        .unwrap();
    assert_eq!(
        store
            .live_durable_session_for_pubkey("pk")
            .unwrap()
            .as_deref(),
        Some("session-b")
    );
}

#[test]
fn concurrent_claims_allow_only_one_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    Store::open(&path).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let threads = ["a", "b"].map(|session| {
        let path = path.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let store = Store::open(&path).unwrap();
            barrier.wait();
            store
                .claim_durable_agent_session("pk", "chief", session, 1)
                .is_ok()
        })
    });
    assert_eq!(
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|claimed| *claimed)
            .count(),
        1
    );
}

#[test]
fn durable_sessions_are_not_resume_candidates_and_dead_pubkeys_do_not_resolve() {
    let store = Store::open_memory().unwrap();
    store
        .claim_durable_agent_session("pk", "chief", "session-a", 1)
        .unwrap();
    store
        .upsert_session_row(
            "session-a",
            &RegisterSession {
                harness: "codex".into(),
                external_id_kind: "harness_session".into(),
                external_id: "native-a".into(),
                agent_pubkey: "pk".into(),
                agent_slug: "chief".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 1,
            },
        )
        .unwrap();
    assert_eq!(
        store.session_for_pubkey("pk").unwrap().unwrap().session_id,
        "session-a"
    );
    assert!(store.list_resumable_sessions(10).unwrap().is_empty());

    store.mark_dead("session-a").unwrap();
    assert!(store.session_for_pubkey("pk").unwrap().is_none());
}

#[test]
fn preexisting_live_same_slug_session_blocks_durable_mode() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_session_row(
            "old-session",
            &RegisterSession {
                harness: "codex".into(),
                external_id_kind: "harness_session".into(),
                external_id: "old-native".into(),
                agent_pubkey: "old-session-pk".into(),
                agent_slug: "chief".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                resume_id: "old-native".into(),
                now: 1,
            },
        )
        .unwrap();
    let error = store
        .claim_durable_agent_session("durable-pk", "chief", "new-session", 2)
        .unwrap_err();
    assert!(error.to_string().contains("already has a live session"));
}

#[test]
fn startup_cleanup_releases_orphan_claim_but_preserves_registered_live_owner() {
    let store = Store::open_memory().unwrap();
    assert!(store
        .claim_durable_agent_session("pk", "chief", "orphan", 1)
        .unwrap());
    assert_eq!(store.cleanup_orphan_durable_sessions().unwrap(), 1);
    assert!(store
        .claim_durable_agent_session("pk", "chief", "live", 2)
        .unwrap());
    store
        .upsert_session_row(
            "live",
            &RegisterSession {
                harness: "codex".into(),
                external_id_kind: "harness_session".into(),
                external_id: "native".into(),
                agent_pubkey: "pk".into(),
                agent_slug: "chief".into(),
                channel_h: "root".into(),
                child_pid: None,
                transcript_path: None,
                resume_id: String::new(),
                now: 2,
            },
        )
        .unwrap();
    assert_eq!(store.cleanup_orphan_durable_sessions().unwrap(), 0);
    assert_eq!(
        store
            .live_durable_session_for_pubkey("pk")
            .unwrap()
            .as_deref(),
        Some("live")
    );
}
