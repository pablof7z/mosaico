//! `<members>` rendering: `@codename-agent` per member.

use crate::fabric_context::{
    assemble, capture_inputs, render_fabric_context, render_fabric_context_human, render_view_text,
};
use crate::state::{RegisterSession, Status, Store};

use super::{input, seed_store, session, session_record, OTHER_PK, SELF_PK};

#[test]
fn empty_status_agents_are_omitted_from_snapshots_and_deltas() {
    let store = seed_store();
    let rec = session(&store);

    let snapshot = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("snapshot should render");
    assert!(!snapshot.contains("<members>"), "got: {snapshot}");
    assert!(!snapshot.contains("status=\"\""), "got: {snapshot}");
    assert!(!snapshot.contains("since=\"unknown\""), "got: {snapshot}");

    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: String::new(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 150,
            last_seen: 150,
            updated_at: 150,
            expiration: 300,
        })
        .unwrap();
    let delta = render_fabric_context(&store, input(Some(&rec), "root", 100, 200, true))
        .expect("forced delta should render");
    assert!(!delta.contains("<recent-presence>"), "got: {delta}");
    assert!(!delta.contains("@amber-reviewer"), "got: {delta}");
}

/// A member with a live status renders under that status's public handle; the
/// The public session handle wins over the durable profile name.
#[test]
fn member_row_shows_session_handle_without_role_for_peer_session() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_profile_with_agent_slug(
            OTHER_PK,
            "amber-reviewer",
            "amber-reviewer",
            "reviewer",
            "laptop",
            false,
            2,
        )
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: "Reviewing".into(),
            activity: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 90,
            last_seen: 90,
            updated_at: 90,
            expiration: 500,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    // The empty-status self row is absent; the peer renders under its public handle.
    assert!(!text.contains("<member ref=\"@coder\""), "got: {text}");
    assert!(
        text.contains("<member ref=\"@amber-reviewer\" state=\"idle\" status=\""),
        "got: {text}"
    );
    assert!(!text.contains("status=\"\""), "got: {text}");
    assert!(!text.contains("since=\"unknown\""), "got: {text}");
    assert!(
        !text.contains(" agentSlug=\""),
        "member rows must not render agentSlug attributes: {text}"
    );
    assert!(
        !text.contains(" role=\""),
        "member rows must not render relay roles: {text}"
    );

    // Parity: the pure capture→assemble path renders byte-identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(rendered, text);
}

#[test]
fn local_lifecycle_overrides_an_expired_offline_relay_echo() {
    let store = seed_store();
    let rec = session(&store);
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: OTHER_PK.into(),
            observed_harness: "codex".into(),
            agent_slug: "reviewer".into(),
            channel_h: "root".into(),
            child_pid: None,
            transcript_path: None,
            now: 95,
        })
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
            title: "Recovered session".into(),
            activity: String::new(),
            state: crate::session_state::SessionState::Offline,
            state_since: 100,
            last_seen: 100,
            updated_at: 100,
            expiration: 100,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 101, true))
        .expect("context should render");
    assert!(
        text.contains(
            "<member ref=\"@reviewer\" state=\"suspended\" status=\"Recovered session\" since=\"just now\""
        ),
        "got: {text}"
    );
    assert!(
        !text.contains("<member ref=\"@reviewer\" state=\"offline\""),
        "local lifecycle must win over its relay echo: {text}"
    );
}

#[test]
fn suspended_and_offline_deltas_match_both_render_paths() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "amber-reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 90,
        last_seen: 90,
        updated_at: 90,
        expiration: 120,
    };
    store.upsert_status(&peer).unwrap();

    let suspended = render_fabric_context(&store, input(Some(&rec), "root", 80, 100, true))
        .expect("suspended delta should render");
    assert!(
        suspended.contains("state=\"suspended\""),
        "got: {suspended}"
    );
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 80, 100, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 80, 100)),
        suspended
    );

    peer.state = crate::session_state::SessionState::Working;
    peer.activity = "stale live activity".into();
    peer.last_seen = 110;
    peer.updated_at = 110;
    store.upsert_status(&peer).unwrap();
    let offline = render_fabric_context(&store, input(Some(&rec), "root", 120, 130, true))
        .expect("expiry delta should render");
    assert!(offline.contains("state=\"offline\""), "got: {offline}");
    assert!(!offline.contains("stale live activity"), "got: {offline}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 120, 130, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 120, 130)),
        offline
    );
}

#[test]
fn lease_renewal_without_state_change_produces_no_presence_delta() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "amber-reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 90,
        last_seen: 90,
        updated_at: 90,
        expiration: 180,
    };
    store.upsert_status(&peer).unwrap();
    peer.last_seen = 150;
    peer.updated_at = 150;
    peer.expiration = 240;
    store.upsert_status(&peer).unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 100, 160, true))
        .expect("forced quiet delta should render");
    assert!(!text.contains("<recent-presence>"), "got: {text}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 100, 160, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 100, 160)),
        text
    );
}

#[test]
fn semantic_status_change_is_a_delta_without_resetting_state_age() {
    let store = seed_store();
    let rec = session(&store);
    let mut peer = Status {
        pubkey: OTHER_PK.into(),
        channel_h: "root".into(),
        slug: "amber-reviewer".into(),
        title: "Reviewing".into(),
        activity: String::new(),
        state: crate::session_state::SessionState::Suspended,
        state_since: 90,
        last_seen: 90,
        updated_at: 90,
        expiration: 240,
    };
    store.upsert_status(&peer).unwrap();
    peer.title = "Updated title".into();
    peer.last_seen = 150;
    peer.updated_at = 150;
    peer.expiration = 300;
    store.upsert_status(&peer).unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 100, 160, true))
        .expect("status-change delta should render");
    assert!(text.contains("<recent-presence>"), "got: {text}");
    assert!(text.contains("text=\"Updated title\""), "got: {text}");
    assert!(text.contains("since=\"1 min ago\""), "got: {text}");
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 100, 160, true)).unwrap();
    assert_eq!(
        render_view_text(&assemble::assemble_view(&captured, 100, 160)),
        text
    );
}

/// Purge guard for the human-facing "project" -> "workspace" rename (#201): a
/// representative agent-facing render must expose the workspace under a
/// `<workspace ...>` element and must NOT leak the word "project" anywhere
/// (case-insensitive). If "project" ever creeps back into agent-facing output
/// this fails loudly.
#[test]
fn agent_render_uses_workspace_and_never_leaks_project() {
    let store = seed_store();
    super::publish_idle_status(&store, OTHER_PK, "reviewer", "Reviewing");
    let rec = session(&store);
    // A representative, non-trivial view: workspace-bearing root channel with
    // members and a channel block.
    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("forced context should render");

    assert!(
        text.contains("<workspace "),
        "agent render must carry a <workspace ...> element; got: {text}"
    );
    assert!(
        text.contains("<member "),
        "expected a members block; got: {text}"
    );
    assert!(
        !text.to_ascii_lowercase().contains("project"),
        "agent-facing render must never contain \"project\"; got: {text}"
    );
}

#[test]
fn same_named_channels_under_different_workspaces_show_workspace_context() {
    let store = Store::open_memory().unwrap();
    store.upsert_channel("test1", "test1", "", "", 1).unwrap();
    store.upsert_channel("test2", "test2", "", "", 1).unwrap();
    store
        .upsert_channel("test1-xxx", "xxx", "", "test1", 2)
        .unwrap();
    store
        .upsert_channel("test2-xxx", "xxx", "", "test2", 2)
        .unwrap();
    store
        .upsert_profile_with_agent_slug(SELF_PK, "coder", "coder", "coder", "laptop", false, 1)
        .unwrap();
    for (pk, slug) in [("peer-test1", "reviewer"), ("peer-test2", "tester")] {
        store
            .upsert_profile_with_agent_slug(pk, slug, slug, slug, "laptop", false, 1)
            .unwrap();
    }
    store
        .replace_channel_members(
            "test1-xxx",
            &[SELF_PK.to_string(), "peer-test1".to_string()],
            3,
        )
        .unwrap();
    store
        .replace_channel_members(
            "test2-xxx",
            &[SELF_PK.to_string(), "peer-test2".to_string()],
            3,
        )
        .unwrap();
    let rec = session_record(&store, "cross-workspace", "test1-xxx");
    store
        .grant_session_route(&rec.pubkey, "test2-xxx", 20)
        .unwrap();
    for (pk, channel, slug, activity) in [
        (
            "peer-test1",
            "test1-xxx",
            "amber-reviewer",
            "checking test1",
        ),
        ("peer-test2", "test2-xxx", "atlas-tester", "checking test2"),
    ] {
        store
            .upsert_status(&Status {
                pubkey: pk.into(),
                channel_h: channel.into(),
                slug: slug.into(),
                title: String::new(),
                activity: activity.into(),
                state: crate::session_state::SessionState::Working,
                state_since: 250,
                last_seen: 250,
                updated_at: 250,
                expiration: 500,
            })
            .unwrap();
    }

    let request = input(Some(&rec), "test1-xxx", 200, 300, true);
    let text = render_fabric_context(&store, request).expect("context should render");
    assert!(
        text.contains("<channel name=\"#xxx\" ref=\"test1.xxx\""),
        "got: {text}"
    );
    assert!(
        text.contains("<channel name=\"#xxx\" ref=\"test2.xxx\""),
        "got: {text}"
    );
    let reviewer = "amber-reviewer";
    let tester = "atlas-tester";
    assert!(
        text.contains(&format!("ref=\"@{reviewer}\"")),
        "got: {text}"
    );
    assert!(text.contains(&format!("ref=\"@{tester}\"")), "got: {text}");

    let captured = capture_inputs(&store, &input(Some(&rec), "test1-xxx", 200, 300, true)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);

    let human = render_fabric_context_human(
        &store,
        input(Some(&rec), "test1-xxx", 200, 300, true),
        false,
    )
    .expect("valid channel ancestry")
    .expect("human context should render");
    assert!(human.contains("#test1.xxx"), "got: {human}");
    assert!(human.contains("#test2.xxx"), "got: {human}");
}
