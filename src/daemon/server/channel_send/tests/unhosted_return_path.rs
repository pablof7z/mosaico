use super::*;

async fn unhosted_state() -> (Arc<DaemonState>, crate::state::Session) {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|store| {
            store.upsert_channel("room", "Room", "", "", 1)?;
            register_session(store, "self-pk", "codex", "room");
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
    let rec = state
        .with_store(|store| store.get_session("self-pk"))
        .unwrap()
        .unwrap();
    (state, rec)
}

#[tokio::test]
async fn warning_is_claimed_only_by_first_risky_directed_action() {
    let (state, rec) = unhosted_state().await;
    let target = vec!["peer-pk".to_string()];

    assert!(
        super::super::unhosted_coaching::maybe_warn(&state, &rec, "room", &target, true, 10,)
            .unwrap()
            .is_none()
    );
    assert!(
        super::super::unhosted_coaching::maybe_warn(&state, &rec, "room", &[], false, 11,)
            .unwrap()
            .is_none()
    );

    let warning =
        super::super::unhosted_coaching::maybe_warn(&state, &rec, "room", &target, false, 12)
            .unwrap()
            .expect("first risky send should coach");
    assert_eq!(warning.code, "unhosted_no_return_path");
    assert!(warning.summary.contains("cannot resume you"));
    assert!(warning.summary.contains("references/unhosted.md"));

    assert!(
        super::super::unhosted_coaching::maybe_warn(&state, &rec, "room", &target, false, 13,)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn hosted_session_never_consumes_unhosted_coaching() {
    let (state, mut rec) = unhosted_state().await;
    rec.admitted_transport = "pty".into();
    let target = vec!["peer-pk".to_string()];

    assert!(
        super::super::unhosted_coaching::maybe_warn(&state, &rec, "room", &target, false, 10,)
            .unwrap()
            .is_none()
    );
}
