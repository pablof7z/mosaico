use super::*;
use crate::state::{TestGroup, TestGroupDelivery};

/// A running, signing session that is a confirmed member of `#room` and has
/// published presence there.
fn joined_session(state: &Arc<DaemonState>) -> String {
    let salt = crate::identity::new_session_signer_salt();
    let keys =
        crate::identity::derive_session_keys(state.backend_keys().secret_key(), &salt).unwrap();
    let pubkey = keys.public_key().to_hex();
    state.with_store(|store| {
        store.install_test_group_delivery(TestGroupDelivery::new([TestGroup::new("room")
            .metadata("room", "", "", 1)
            .members([pubkey.clone()])]));
        store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: pubkey.clone(),
                    observed_harness: "codex".into(),
                    agent_slug: "codex".into(),
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    preset: String::new(),
                    transport: "pty".into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
        store.bind_session_signer(&pubkey, &salt).unwrap();
        let session = store.get_session(&pubkey).unwrap().unwrap();
        store
            .commit_confirmed_session_admission(
                &pubkey,
                "room",
                session.runtime_generation,
                session.lifecycle_epoch,
                1,
            )
            .unwrap();
    });
    let session = state.with_store(|store| store.get_session(&pubkey).unwrap().unwrap());
    let projection = state.with_store(|store| {
        crate::session_presence::publication_with_extension(store, &session, false)
    });
    assert!(projection.channels.contains("room"));
    state.reconcilers.status.lock().unwrap().open(
        &pubkey,
        session.runtime_generation,
        crate::reconcile::PresenceSnapshot {
            host: "test-host".into(),
            workspace: "room".into(),
            slug: "codex".into(),
            rel_cwd: ".".into(),
            dispatch_event: None,
            projection,
        },
        1,
    );
    pubkey
}

/// A join that re-admits the channel while the leave's presence withdrawal
/// is still publishing supersedes the leave: it reports `left: false` with
/// a reason, keeps the joined route, and never removes relay membership.
#[tokio::test]
async fn a_join_during_the_withdrawal_supersedes_the_leave() {
    let state = DaemonState::new_for_test().await;
    let pubkey = joined_session(&state);

    // The withdrawal's awaited publication cannot start while the lane is
    // held: a deterministic stand-in for a wedged relay.
    let lane = state
        .reconcilers
        .presence_publisher
        .hold_lane_for_test()
        .await;
    let leave = tokio::spawn({
        let state = state.clone();
        let params = serde_json::json!({ "channel": "#room", "session": &pubkey });
        async move { rpc_channel_leave(&state, &params).await }
    });
    // Phase one has removed the route.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while state.with_store(|store| store.has_session_route(&pubkey, "room").unwrap()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("leave never began");
    assert!(!leave.is_finished());

    // The concurrent join: still a relay member, so it commits locally.
    let session = state.with_store(|store| store.get_session(&pubkey).unwrap().unwrap());
    ensure_joinable(&state, &session, "room")
        .await
        .expect("rejoin while the leave is withdrawing");
    drop(lane);

    let reply = tokio::time::timeout(std::time::Duration::from_secs(10), leave)
        .await
        .expect("leave finished")
        .unwrap()
        .expect("a superseded leave is not an error");
    assert_eq!(reply["left"], false);
    assert_eq!(reply["reason"], "superseded_by_join");
    assert!(state.with_store(|store| store.has_session_route(&pubkey, "room").unwrap()));
    assert_eq!(
        state.with_store(|store| store
            .get_session_standing(&pubkey, "room")
            .unwrap()
            .unwrap()
            .state),
        crate::state::StandingState::Member
    );
}
