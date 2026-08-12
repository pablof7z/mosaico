//! The human/terminal projection of the same fabric view the agents get.

use crate::fabric_context::render_fabric_context_human;

use super::{
    chat, idle_status, input, seed_profiles, seed_store, session, TestRelayDelivery, SELF_PK,
};

#[test]
fn human_who_renderer_is_non_xml_and_terminal_friendly() {
    let store = seed_store();
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(SELF_PK, "coder", "Reviewing fabric context")]),
    );

    let human = render_fabric_context_human(&store, input(None, "root", 0, 1_000, true), false)
        .expect("valid channel ancestry")
        .expect("human who should render");

    assert!(human.starts_with("#root\nRoot room\n\n"), "got: {human}");
    assert!(human.contains("#root/task"), "got: {human}");
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
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(SELF_PK, "coder", "Reviewing fabric context")]),
    );

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
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new()
            .profiles(seed_profiles())
            .statuses([idle_status(SELF_PK, "coder", "Reviewing fabric context")])
            .events([chat("msg-1", "root", 400, "on it", "[]")]),
    );
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
