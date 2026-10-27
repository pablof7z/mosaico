//! Presence transitions in turn deltas: a member becoming present, a dormant
//! member leaving quietly, and a lapsed heartbeat announced as offline.

use super::*;
use crate::reconcile::HookContextState;

/// A dormant member that becomes present again (a live status appears) is
/// announced through the ordinary member delta.
#[test]
fn a_dormant_agent_that_becomes_present_emits_a_member_delta() {
    let store = seed_store();
    let rec = session(&store);
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.install_test_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()).statuses(
        [crate::state::Status {
            state_since: 250,
            last_seen: 250,
            updated_at: 250,
            ..idle_status(OTHER_PK, "reviewer", "Back on review")
        }],
    ));
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("a member becoming present is a delta");
    assert!(delta.contains("<agent name=\"@reviewer\""), "{delta}");
}

/// A dormant member was never listed, so its removal from the roster is not
/// announced as a departure either.
#[test]
fn a_dormant_member_leaving_the_roster_is_not_announced() {
    let store = seed_store();
    let rec = session(&store);
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.install_test_group_delivery(TestGroupDelivery::new([
        root_group_with_roster(&[SELF_PK], &[]),
        task_group(),
    ]));
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let text = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .unwrap_or_default();
    assert!(!text.contains("reviewer"), "{text}");
}

/// A heartbeat that lapses without a closing status (a crash) drops out of
/// the view; the member is announced as going offline, not silently dropped
/// and not as a departure, since its membership is unchanged.
#[test]
fn a_lapsed_heartbeat_is_announced_as_offline() {
    let store = seed_store();
    let rec = session(&store);
    store.install_test_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(OTHER_PK, "reviewer", "Reviewing")]),
    );
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.install_test_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()));
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("a lapsed heartbeat is a delta");
    assert!(delta.contains("@reviewer"), "{delta}");
    assert!(delta.contains("offline"), "{delta}");
    assert!(!delta.contains("left."), "{delta}");
}

/// An untitled member's lapse is announced too: an offline row needs no text.
#[test]
fn an_untitled_lapsed_heartbeat_is_announced_as_offline() {
    let store = seed_store();
    let rec = session(&store);
    store.install_test_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()).statuses(
        [crate::state::Status {
            title: String::new(),
            ..idle_status(OTHER_PK, "reviewer", "")
        }],
    ));
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.install_test_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()));
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("an untitled lapse is still a delta");
    assert!(delta.contains("@reviewer"), "{delta}");
    assert!(delta.contains("offline"), "{delta}");
}

/// A running local session whose heartbeat lapses (a stalled publisher) is
/// announced offline in the turn delta, not relabelled present by lifecycle.
#[test]
fn a_running_local_session_whose_heartbeat_lapses_is_announced_offline() {
    let store = seed_store();
    let rec = session(&store);
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: OTHER_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "reviewer".into(),
            launch_channel_h: "root".into(),
            work_root: "root".into(),
            child_pid: None,
            now: 150,
        })
        .unwrap();
    store.install_test_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(OTHER_PK, "reviewer", "Reviewing")]),
    );
    let mut hook = HookContextState::default();
    let before = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    hook.render_context(&rec.pubkey, "turn_start", 200, 300, before);

    store.install_test_relay_delivery(TestRelayDelivery::new().profiles(seed_profiles()));
    let after = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let delta = hook
        .render_context(&rec.pubkey, "turn_start", 200, 300, after)
        .text
        .expect("a stalled local heartbeat is a delta");
    assert!(delta.contains("@reviewer"), "{delta}");
    assert!(delta.contains("offline"), "{delta}");
}
