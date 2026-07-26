//! The human/terminal projection of the same fabric view the agents get.

use crate::fabric_context::render_fabric_context_human;

use super::{input, publish_idle_status, seed_store, session, OTHER_PK, SELF_PK};

#[test]
fn human_who_renderer_is_non_xml_and_terminal_friendly() {
    let store = seed_store();
    publish_idle_status(&store, SELF_PK, "coder", "Reviewing fabric context");

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), false)
        .expect("valid channel ancestry")
        .expect("human who should render");

    assert!(human.starts_with("/root\nRoot room\n\n"), "got: {human}");
    assert!(human.contains("/root/task"), "got: {human}");
    assert!(human.contains("Members"), "got: {human}");
    assert!(human.contains("@coder"), "got: {human}");
    assert!(human.contains("idle"), "got: {human}");
    assert!(!human.contains(" member "), "got: {human}");
    assert!(!human.contains("<mosaico>"), "got: {human}");
    assert!(!human.contains("<member"), "got: {human}");
}

#[test]
fn human_who_renderer_colorizes_when_requested() {
    let store = seed_store();
    publish_idle_status(&store, SELF_PK, "coder", "Reviewing fabric context");

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), true)
        .expect("valid channel ancestry")
        .expect("human who should render");

    assert!(
        human.contains("\u{1b}["),
        "expected ansi styling: {human:?}"
    );
    assert!(human.contains("@coder"), "got: {human}");
}

/// A member surfaced from message activity alone has no lifecycle state, so the
/// terminal row simply omits the state word rather than printing a placeholder.
#[test]
fn a_member_without_a_heartbeat_renders_with_no_state_word() {
    let store = seed_store();
    publish_idle_status(&store, SELF_PK, "coder", "Reviewing fabric context");
    store
        .insert_event(&crate::state::RelayEvent {
            id: "msg-1".into(),
            kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
            pubkey: OTHER_PK.into(),
            created_at: 400,
            channel_h: "root".into(),
            d_tag: String::new(),
            content: "on it".into(),
            tags_json: "[]".into(),
        })
        .unwrap();
    let rec = session(&store);

    let human =
        render_fabric_context_human(&store, input(Some(&rec), "root", 0, 1_000, true), false)
            .expect("valid channel ancestry")
            .expect("human who should render");

    let row = human
        .lines()
        .find(|line| line.contains("@reviewer"))
        .unwrap_or_else(|| panic!("reviewer row missing: {human}"));
    assert!(row.contains("since"), "got: {row}");
    for state in ["idle", "offline", "working", "suspended"] {
        assert!(!row.contains(state), "no state word expected in {row:?}");
    }
}
