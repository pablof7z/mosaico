//! Typed `<members>` rendering from the full roster.

use crate::fabric_context::{
    assemble, capture_inputs, render_fabric_context, render_fabric_context_human, render_view_text,
};
use crate::state::{RegisterSession, Status, Store, TestGroup, TestGroupDelivery};

use super::{
    idle_status, input, profile, pubkeys, seed_profiles, seed_store, session, session_record,
    TestRelayDelivery, OTHER_PK, SELF_PK,
};
mod lifecycle;
#[path = "member_render/native_outcome.rs"]
mod native_outcome;

#[test]
fn full_snapshot_keeps_nameable_members_while_empty_presence_deltas_stay_quiet() {
    let store = seed_store();
    let rec = session(&store);

    let snapshot = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("snapshot should render");
    assert!(snapshot.contains("<members>"), "got: {snapshot}");
    // Membership itself is relevant awareness. Unknown lifecycle facts are
    // omitted instead of hiding the nameable member.
    assert!(
        snapshot.contains("<agent name=\"@reviewer\" />"),
        "got: {snapshot}"
    );
    assert!(!snapshot.contains("since=\"unknown\""), "got: {snapshot}");
    assert!(!snapshot.contains("status=\"\""), "got: {snapshot}");

    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([Status {
                pubkey: OTHER_PK.into(),
                channel_h: "root".into(),
                slug: "amber-reviewer".into(),
                title: String::new(),
                activity: String::new(),
                workspace: "root".into(),
                branch: String::new(),
                state: crate::session_state::SessionState::Idle,
                state_since: 150,
                last_seen: 150,
                updated_at: 150,
                expiration: 300,
            }]),
    );
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
    let mut profiles = seed_profiles();
    profiles.retain(|candidate| candidate.pubkey != OTHER_PK);
    let mut peer_profile = profile(
        OTHER_PK,
        "amber-reviewer",
        "amber-reviewer",
        "reviewer",
        "laptop",
        false,
    );
    peer_profile.updated_at = 2;
    profiles.push(peer_profile);
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles(profiles).statuses([
        Status {
            pubkey: OTHER_PK.into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: "Reviewing".into(),
            activity: String::new(),
            workspace: "root".into(),
            branch: String::new(),
            state: crate::session_state::SessionState::Idle,
            state_since: 90,
            last_seen: 90,
            updated_at: 90,
            expiration: 500,
        },
    ]));

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    // The full roster is retained; the peer renders under its public handle.
    assert!(text.contains("<agent name=\"@coder\""), "got: {text}");
    assert!(
        text.contains("<agent name=\"@amber-reviewer\" state=\"idle\" status=\""),
        "got: {text}"
    );
    assert!(!text.contains("status=\"\""), "got: {text}");
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
fn lease_renewal_without_state_change_produces_no_presence_delta() {
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
        expiration: 180,
    };
    peer.last_seen = 150;
    peer.updated_at = 150;
    peer.expiration = 240;
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([peer]),
    );

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
fn agent_render_includes_workspace_and_members() {
    let store = seed_store();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(OTHER_PK, "reviewer", "Reviewing")]),
    );
    let rec = session(&store);
    // A representative, non-trivial view: workspace-bearing root channel with
    // members and a channel block.
    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("forced context should render");

    assert!(
        text.contains("workspace=\"root\""),
        "agent render must carry the immutable self workspace; got: {text}"
    );
    assert!(
        text.contains("<agent "),
        "expected a members block; got: {text}"
    );
}

#[test]
fn same_named_channels_under_different_workspaces_show_workspace_context() {
    let store = Store::open_memory().unwrap();
    store.install_test_nmp_group_delivery(TestGroupDelivery::new([
        TestGroup::new("test1").metadata("test1", "", "", 1),
        TestGroup::new("test2").metadata("test2", "", "", 1),
        TestGroup::new("test1-xxx")
            .metadata("xxx", "", "test1", 2)
            .admins(Vec::new())
            .members(pubkeys(&[SELF_PK, "peer-test1"])),
        TestGroup::new("test2-xxx")
            .metadata("xxx", "", "test2", 2)
            .admins(Vec::new())
            .members(pubkeys(&[SELF_PK, "peer-test2"])),
    ]));
    let profiles = [
        profile(SELF_PK, "coder", "coder", "coder", "laptop", false),
        profile(
            "peer-test1",
            "reviewer",
            "reviewer",
            "reviewer",
            "laptop",
            false,
        ),
        profile("peer-test2", "tester", "tester", "tester", "laptop", false),
    ];
    let rec = session_record(&store, "cross-workspace", "test1-xxx");
    store
        .grant_session_route(&rec.pubkey, "test2-xxx", 20)
        .unwrap();
    let statuses = [
        (
            "peer-test1",
            "test1-xxx",
            "amber-reviewer",
            "checking test1",
        ),
        ("peer-test2", "test2-xxx", "atlas-tester", "checking test2"),
    ]
    .into_iter()
    .map(|(pk, channel, slug, activity)| Status {
        pubkey: pk.into(),
        channel_h: channel.into(),
        slug: slug.into(),
        title: String::new(),
        activity: activity.into(),
        workspace: channel.split('-').next().unwrap_or(channel).into(),
        branch: format!("feat/{}", channel.split('-').next().unwrap_or(channel)),
        state: crate::session_state::SessionState::Working,
        state_since: 250,
        last_seen: 250,
        updated_at: 250,
        expiration: 500,
    })
    .collect::<Vec<_>>();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(profiles)
            .statuses(statuses),
    );

    let request = input(Some(&rec), "test1-xxx", 200, 300, true);
    let text = render_fabric_context(&store, request).expect("context should render");
    assert!(text.contains("<channel name=\"#test1/xxx\""), "got: {text}");
    assert!(text.contains("<channel name=\"#test2/xxx\""), "got: {text}");
    let reviewer = "amber-reviewer";
    let tester = "atlas-tester";
    assert!(
        text.contains(&format!("name=\"@{reviewer}\"")),
        "got: {text}"
    );
    assert!(text.contains(&format!("name=\"@{tester}\"")), "got: {text}");
    assert!(
        text.contains(&format!(
            "name=\"@{tester}\" host=\"laptop\" workspace=\"test2\" branch=\"feat/test2\""
        )),
        "cross-workspace agents need explicit non-duplicated origin: {text}"
    );
    assert!(!text.contains("@atlas-tester@laptop"), "got: {text}");

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
    assert!(human.contains("#test1/xxx"), "got: {human}");
    assert!(human.contains("#test2/xxx"), "got: {human}");
}
