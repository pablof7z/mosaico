use super::WhoAggregation;
use crate::state::{RegisterSession, Status, StopReason, Store, TestRelayDelivery};

fn register(store: &Store, pubkey: &str) {
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pubkey.into(),
            observed_harness: "codex".into(),
            agent_slug: "codex".into(),
            launch_channel_h: "room".into(),
            work_root: "room".into(),
            child_pid: Some(42),
            now: 1000,
        })
        .unwrap();
    // A derived session: its handle comes from a lease, which none of these has.
    store.bind_session_signer(pubkey, "salt").unwrap();
}

#[test]
fn a_stopped_session_whose_lease_was_reclaimed_loads_without_an_identity() {
    let store = Store::open_memory().unwrap();
    register(&store, "stopped");
    store
        .mark_runtime_stopped("stopped", StopReason::Unknown, 1001)
        .unwrap();
    // A leftover status makes `who` reference the stopped session.
    store.install_test_relay_delivery(TestRelayDelivery::new().statuses([Status {
        pubkey: "stopped".into(),
        channel_h: "room".into(),
        slug: "codex".into(),
        title: String::new(),
        activity: String::new(),
        workspace: String::new(),
        branch: String::new(),
        state: crate::session_state::SessionState::Idle,
        state_since: 1,
        last_seen: 1,
        updated_at: 1,
        expiration: 0,
    }]));

    let aggregation = WhoAggregation::load(&store).unwrap();
    assert!(aggregation.local_sessions.is_empty());
    assert!(aggregation.session_identity("stopped").is_none());
}

#[test]
fn a_running_session_without_a_lease_is_an_error() {
    let store = Store::open_memory().unwrap();
    register(&store, "running");

    let error = WhoAggregation::load(&store).err().unwrap();
    assert!(error.to_string().contains("has no handle lease"), "{error:#}");
}
