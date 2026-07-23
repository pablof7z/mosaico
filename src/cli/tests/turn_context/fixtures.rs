use crate::state::{Session, Status, Store};

pub(super) const BACKEND: &str = "pk-backend";

/// Publish a relay_status (kind:30315) row — the single source awareness reads
/// for "who is doing what here", local and remote alike.
#[allow(clippy::too_many_arguments)]
pub(super) fn pub_status(
    store: &Store,
    pubkey: &str,
    slug: &str,
    title: &str,
    activity: &str,
    busy: bool,
    updated_at: u64,
    now: u64,
) {
    store
        .upsert_status(&Status {
            pubkey: pubkey.to_string(),
            channel_h: "proj".to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            activity: activity.to_string(),
            state: if busy {
                crate::session_state::SessionState::Working
            } else {
                crate::session_state::SessionState::Idle
            },
            state_since: updated_at,
            last_seen: updated_at,
            updated_at,
            expiration: now + 90,
        })
        .unwrap();
}

/// Materialize the `proj` channel + roster so awareness has fabric context.
pub(super) fn seed_channel(store: &Store) {
    // Opaque id "proj" with a distinct human name "main" (production ids are random, never the name).
    store.upsert_channel("proj", "main", "", "", 1).unwrap();
    store
        .replace_channel_members("proj", &["pk-coder".to_string()], 1)
        .unwrap();
    store
        .upsert_profile_with_agent_slug("pk-coder", "coder", "coder", "coder", "laptop", false, 1)
        .unwrap();
    store
        .reserve_hook_session_for_test(&crate::state::RegisterSession {
            pubkey: "pk-coder".to_string(),
            observed_harness: "claude-code".to_string(),
            agent_slug: "coder".to_string(),
            channel_h: "proj".to_string(),
            child_pid: None,
            transcript_path: None,
            now: 1,
        })
        .unwrap();
}

pub(super) fn test_session(_id: &str) -> Session {
    Session {
        pubkey: "pk-coder".to_string(),
        runtime_generation: 1,
        agent_slug: "coder".to_string(),
        channel_h: "proj".to_string(),
        work_root: "proj".to_string(),
        readiness_parent: String::new(),
        observed_harness: "claude-code".to_string(),
        claimed_harness: String::new(),
        admitted_bundle: String::new(),
        admitted_transport: String::new(),
        endpoint_provenance: "hook".to_string(),
        child_pid: None,
        transcript_path: None,
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
        explicit_chat_published_at: 0,
        state_changed_at: 1,
    }
}
