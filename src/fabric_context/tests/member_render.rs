//! `<members>` rendering: `@codename-agent` per member, with the legacy (`people`)
//! and pure (`assemble`) paths proven byte-identical.

use crate::fabric_context::{
    assemble, capture_inputs, render_fabric_context, render_fabric_context_human, render_view_text,
};
use crate::state::{Status, Store};

use super::{input, seed_store, session, session_record, OTHER_PK, SELF_PK};

/// A member whose session is known (a live status carries its session id) renders
/// as `@codename-agent`; the pure and legacy paths agree.
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
            session_id: "peer-sess".into(),
            channel_h: "root".into(),
            slug: "amber-reviewer".into(),
            title: "Reviewing".into(),
            activity: String::new(),
            busy: false,
            last_seen: 90,
            updated_at: 90,
            expiration: 500,
        })
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 0, 100, true))
        .expect("context should render");
    // Self keeps its fallback slug ref; the peer session renders under its public handle.
    assert!(
        text.contains("<member ref=\"@coder\" status=\""),
        "got: {text}"
    );
    assert!(
        text.contains("<member ref=\"@amber-reviewer\" status=\""),
        "got: {text}"
    );
    assert!(
        !text.contains(" agentSlug=\""),
        "member rows must not render agentSlug attributes: {text}"
    );
    assert!(
        !text.contains(" role=\""),
        "member rows must not render relay roles: {text}"
    );

    // Parity: the pure capture→assemble path renders byte-identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 0, 100, true));
    let trellis = render_view_text(&assemble::assemble_view(&captured, 0, 100));
    assert_eq!(trellis, text);
}

/// Purge guard for the human-facing "project" -> "workspace" rename (#201): a
/// representative agent-facing render must expose the workspace under a
/// `<workspace ...>` element and must NOT leak the word "project" anywhere
/// (case-insensitive). If "project" ever creeps back into agent-facing output
/// this fails loudly.
#[test]
fn agent_render_uses_workspace_and_never_leaks_project() {
    let store = seed_store();
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
        .join_session_channel(&rec.session_id, "test2-xxx", 20)
        .unwrap();
    for (pk, session_id, channel, slug, activity) in [
        (
            "peer-test1",
            "peer-test1-session",
            "test1-xxx",
            "amber-reviewer",
            "checking test1",
        ),
        (
            "peer-test2",
            "peer-test2-session",
            "test2-xxx",
            "atlas-tester",
            "checking test2",
        ),
    ] {
        store
            .upsert_status(&Status {
                pubkey: pk.into(),
                session_id: session_id.into(),
                channel_h: channel.into(),
                slug: slug.into(),
                title: String::new(),
                activity: activity.into(),
                busy: true,
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

    let captured = capture_inputs(&store, &input(Some(&rec), "test1-xxx", 200, 300, true));
    let trellis = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(trellis, text);

    let human = render_fabric_context_human(
        &store,
        input(Some(&rec), "test1-xxx", 200, 300, true),
        false,
    )
    .expect("human context should render");
    assert!(human.contains("#test1.xxx"), "got: {human}");
    assert!(human.contains("#test2.xxx"), "got: {human}");
}
