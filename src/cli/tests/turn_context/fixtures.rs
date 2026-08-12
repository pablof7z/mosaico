use crate::state::{
    Profile, RelayEvent, Session, Status, Store, TestGroup, TestGroupDelivery, TestRelayDelivery,
};

pub(super) const BACKEND: &str = "pk-backend";

#[allow(clippy::too_many_arguments)]
pub(super) fn observed_status(
    pubkey: &str,
    slug: &str,
    title: &str,
    activity: &str,
    busy: bool,
    updated_at: u64,
    now: u64,
) -> Status {
    Status {
        pubkey: pubkey.to_string(),
        channel_h: "proj".to_string(),
        slug: slug.to_string(),
        title: title.to_string(),
        activity: activity.to_string(),
        workspace: String::new(),
        branch: String::new(),
        state: if busy {
            crate::session_state::SessionState::Working
        } else {
            crate::session_state::SessionState::Idle
        },
        state_since: updated_at,
        last_seen: updated_at,
        updated_at,
        expiration: now + 90,
    }
}

pub(super) fn install_relay_delivery(
    store: &Store,
    statuses: impl IntoIterator<Item = Status>,
    events: impl IntoIterator<Item = RelayEvent>,
) {
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles([Profile {
                pubkey: "pk-coder".into(),
                name: "coder".into(),
                slug: "coder".into(),
                agent_slug: "coder".into(),
                host: "laptop".into(),
                is_backend: false,
                agents: Vec::new(),
                workspaces: Vec::new(),
                updated_at: 1,
            }])
            .statuses(statuses)
            .events(events),
    );
}

/// Install the `proj` group and a complete empty NMP row delivery.
pub(super) fn seed_channel(store: &Store) {
    // Opaque id "proj" with a distinct human name "main" (production ids are random, never the name).
    install_channel_delivery(store, ["pk-coder".to_string()]);
    install_relay_delivery(store, [], []);
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: "pk-coder".to_string(),
            observed_harness: "claude-code".to_string(),
            agent_slug: "coder".to_string(),
            launch_channel_h: "proj".to_string(),
            work_root: "proj".to_string(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
}

pub(super) fn install_channel_delivery(store: &Store, members: impl IntoIterator<Item = String>) {
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([TestGroup::new("proj")
        .metadata("main", "", "", 1)
        .admins(Vec::new())
        .members(members)]));
}

pub(super) fn test_session(_id: &str) -> Session {
    Session {
        pubkey: "pk-coder".to_string(),
        runtime_generation: 1,
        agent_slug: "coder".to_string(),
        work_root: "proj".to_string(),
        readiness_parent: String::new(),
        observed_harness: "claude-code".to_string(),
        claimed_harness: String::new(),
        admitted_bundle: String::new(),
        admitted_transport: String::new(),
        endpoint_provenance: "hook".to_string(),
        child_pid: None,
        runtime_state: crate::state::RuntimeState::Running,
        presentation_state: crate::state::PresentationState::Headed,
        work_state: crate::state::WorkState::Idle,
        recovery_state: crate::state::RecoveryState::Pending,
        lifecycle_epoch: 1,
        attachment_epoch: 1,
        idle_since: 0,
        idle_deadline: 0,
        stopped_at: 0,
        stop_reason: None,
        turn_count: 0,
        busy_seconds: 0,
        created_at: 1,
        last_seen: 1,
        turn_started_at: 0,
        seen_cursor: 0,
        title: String::new(),
        state_changed_at: 1,
    }
}
