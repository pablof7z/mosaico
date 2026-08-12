use super::*;

#[test]
fn local_lifecycle_overrides_an_offline_relay_row() {
    let store = seed_store();
    let rec = session(&store);
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: OTHER_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "reviewer".into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: None,
            now: 95,
        })
        .unwrap();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([Status {
                pubkey: OTHER_PK.into(),
                channel_h: "root".into(),
                slug: "reviewer".into(),
                title: "Recovered session".into(),
                activity: String::new(),
                workspace: "root".into(),
                branch: String::new(),
                state: crate::session_state::SessionState::Offline,
                state_since: 100,
                last_seen: 100,
                updated_at: 100,
                expiration: 100,
            }]),
    );

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 101, true))
        .expect("context should render");
    assert!(
        text.contains(
            "<agent name=\"@reviewer\" state=\"suspended\" status=\"Recovered session\" since=\"just now\""
        ),
        "got: {text}"
    );
}

#[test]
fn suspended_and_working_deltas_follow_the_current_row() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "amber-reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        workspace: "root".into(),
        branch: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 90,
        last_seen: 90,
        updated_at: 90,
        expiration: 120,
    };
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([peer.clone()]),
    );

    let suspended = render_fabric_context(&store, input(Some(&rec), "root", 80, 100, true))
        .expect("suspended delta should render");
    assert!(
        suspended.contains("state=\"suspended\""),
        "got: {suspended}"
    );
    assert!(suspended.contains("<members>"), "got: {suspended}");
    assert!(suspended.contains("<agent name="), "got: {suspended}");
    assert!(!suspended.contains("<status "), "got: {suspended}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 80, 100, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 80, 100)),
        suspended
    );

    peer.state = crate::session_state::SessionState::Working;
    peer.activity = "stale live activity".into();
    peer.last_seen = 110;
    peer.updated_at = 110;
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([peer]),
    );
    let working = render_fabric_context(&store, input(Some(&rec), "root", 100, 130, true))
        .expect("current status delta should render");
    assert!(working.contains("state=\"working\""), "got: {working}");
    assert!(working.contains("stale live activity"), "got: {working}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 100, 130, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 100, 130)),
        working
    );
}
