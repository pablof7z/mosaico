use super::*;

fn seed() -> (Store, Session) {
    let store = Store::open_memory().unwrap();
    store
        .reserve_session_with_facts(
            &RegisterSession {
                pubkey: "pk".into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                channel_h: "room".into(),
                child_pid: Some(42),
                transcript_path: None,
                now: 1,
            },
            &AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                bundle: "codex-pty".into(),
                transport: "pty".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    let session = store.get_session("pk").unwrap().unwrap();
    (store, session)
}

#[test]
fn unavailable_probe_clears_idle_without_inventing_attachment_epoch() {
    let (store, initial) = seed();
    assert!(store
        .apply_session_presentation_edge(
            "pk",
            initial.runtime_generation,
            2,
            PresentationState::Headless,
            10,
        )
        .unwrap());
    assert!(store
        .mark_session_presentation_unavailable("pk", initial.runtime_generation, 2, 20)
        .unwrap());

    let unavailable = store.get_session("pk").unwrap().unwrap();
    assert_eq!(
        unavailable.presentation_state,
        PresentationState::Unavailable
    );
    assert_eq!(unavailable.attachment_epoch, 2);
    assert_eq!(unavailable.idle_since, 0);
    assert_eq!(unavailable.idle_deadline, 0);
    assert!(!store
        .mark_session_presentation_unavailable("pk", initial.runtime_generation, 1, 30)
        .unwrap());
}
