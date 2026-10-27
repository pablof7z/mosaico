use super::*;
use crate::state::{Profile, TestRelayDelivery};

fn profile(pubkey: &str, name: &str, slug: &str, agent_slug: &str) -> Profile {
    Profile {
        pubkey: pubkey.into(),
        name: name.into(),
        slug: slug.into(),
        agent_slug: agent_slug.into(),
        host: "host".into(),
        is_backend: false,
        agents: Vec::new(),
        workspaces: Vec::new(),
        updated_at: 1,
    }
}

fn live_status(pubkey: &str) -> crate::state::Status {
    crate::state::Status {
        pubkey: pubkey.into(),
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
    }
}

fn facts(
    is_admin: bool,
    is_backend: bool,
    has_profile: bool,
    is_named_agent: bool,
    is_live: bool,
) -> MemberFacts {
    MemberFacts {
        is_admin,
        is_backend,
        has_profile,
        is_named_agent,
        is_live,
    }
}

#[test]
fn canonical_member_count_matrix_preserves_unknown_semantics() {
    let cases = [
        (
            "unhydrated",
            false,
            vec![facts(false, false, true, true, true)],
            None,
        ),
        (
            "backend",
            true,
            vec![facts(false, true, false, false, false)],
            Some(0),
        ),
        (
            "admin",
            true,
            vec![facts(true, false, false, true, true)],
            Some(0),
        ),
        (
            "live named agent",
            true,
            vec![facts(false, false, false, true, true)],
            Some(1),
        ),
        (
            "dormant named agent keeps membership but is not counted",
            true,
            vec![facts(false, false, false, true, false)],
            Some(0),
        ),
        (
            "906 dormant members and one live agent",
            true,
            (0..906)
                .map(|_| facts(false, false, true, true, false))
                .chain([facts(false, false, true, true, true)])
                .collect(),
            Some(1),
        ),
        (
            "known human with handle",
            true,
            vec![facts(false, false, true, false, false)],
            Some(0),
        ),
        (
            "known human with empty slug",
            true,
            vec![facts(false, false, true, false, false)],
            Some(0),
        ),
        (
            "unknown identity",
            true,
            vec![facts(false, false, false, false, false)],
            None,
        ),
    ];

    for (name, hydrated, members, expected) in cases {
        assert_eq!(count_agents(hydrated, members), expected, "{name}");
    }
}

#[test]
fn normalized_store_matrix_is_shared_by_both_projection_surfaces() {
    let store = Store::open_memory().unwrap();
    store.install_test_relay_delivery(
        TestRelayDelivery::new()
            .statuses([live_status("agent")])
            .profiles([
                profile("agent", "Agent", "agent", "codex"),
                profile("dormant", "Dormant", "dormant", "codex"),
                profile("human", "Human", "human", ""),
                profile("empty-human", "Human", "", ""),
            ]),
    );
    let index = MemberFactIndex::capture(&store, "backend").unwrap();
    let member = |pubkey: &str, role: &str| ChannelMember {
        channel_h: "room".into(),
        pubkey: pubkey.into(),
        role: role.into(),
    };
    let cases = [
        (
            "backend recovery identity",
            member("backend", "member"),
            Some(0),
        ),
        ("unknown admin", member("admin", "admin"), Some(0)),
        ("live named agent", member("agent", "member"), Some(1)),
        ("dormant named agent", member("dormant", "member"), Some(0)),
        ("known human", member("human", "member"), Some(0)),
        (
            "known human with empty slug",
            member("empty-human", "member"),
            Some(0),
        ),
        ("unknown identity", member("unknown", "member"), None),
    ];

    for (name, member, expected) in cases {
        let normalized = index.normalize(&store, &member).unwrap();
        assert_eq!(count_agents(true, [normalized]), expected, "{name}");
        assert_eq!(count_agents(false, [normalized]), None, "{name} cold");
    }
}

#[test]
fn local_lifecycle_decides_liveness_for_sessions_this_daemon_owns() {
    let store = Store::open_memory().unwrap();
    let register = |pubkey: &str| crate::state::RegisterSession {
        pubkey: pubkey.into(),
        observed_harness: "codex".into(),
        agent_slug: "codex".into(),
        launch_channel_h: "room".into(),
        work_root: "room".into(),
        child_pid: Some(42),
        now: 1000,
    };
    store
        .reserve_hook_session_for_test(&register("running"))
        .unwrap();
    store
        .reserve_hook_session_for_test(&register("stopped"))
        .unwrap();
    store
        .mark_runtime_stopped("stopped", crate::state::StopReason::Unknown, 1001)
        .unwrap();
    // The stopped session's last live status is still in the view; local
    // lifecycle overrides it for a session this daemon owns.
    store.install_test_relay_delivery(TestRelayDelivery::new().statuses([live_status("stopped")]));
    let index = MemberFactIndex::capture(&store, "backend").unwrap();
    let count = |pubkey: &str| {
        let member = ChannelMember {
            channel_h: "room".into(),
            pubkey: pubkey.into(),
            role: "member".into(),
        };
        count_agents(true, [index.normalize(&store, &member).unwrap()])
    };

    assert_eq!(count("running"), Some(1));
    assert_eq!(count("stopped"), Some(0));
}
