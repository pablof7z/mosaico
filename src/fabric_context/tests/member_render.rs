//! `<members>` rendering: `@agent/codename` per member, with the legacy (`people`)
//! and pure (`assemble`) paths proven byte-identical.

use crate::fabric_context::{assemble, capture_inputs, render_fabric_context, render_view_text};
use crate::state::Status;

use super::{input, seed_store, session, OTHER_PK};

/// A member whose session is known (a live status carries its session id) renders
/// as `@agent/codename`; the pure and legacy paths agree.
#[test]
fn member_row_shows_session_handle_without_role_for_peer_session() {
    let store = seed_store();
    let rec = session(&store);
    store
        .upsert_profile_with_agent_slug(
            OTHER_PK, "reviewer", "reviewer", "reviewer", "laptop", false, 2,
        )
        .unwrap();
    store
        .upsert_status(&Status {
            pubkey: OTHER_PK.into(),
            session_id: "peer-sess".into(),
            channel_h: "root".into(),
            slug: "reviewer".into(),
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
        text.contains("<member ref=\"@coder\" agentSlug=\"coder\" status=\""),
        "got: {text}"
    );
    let peer_codename = crate::util::friendly_short_code("peer-sess");
    assert!(
        text.contains(&format!(
            "<member ref=\"@reviewer/{peer_codename}\" agentSlug=\"reviewer\" status=\""
        )),
        "got: {text}"
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
