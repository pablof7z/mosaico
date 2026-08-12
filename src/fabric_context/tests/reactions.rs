//! Turn-start reaction awareness: a materialized kind:7 on the caller's own
//! message renders exactly once, is cursor-gated so it never repeats, ignores
//! backend (daemon 👁 receipt) reactors, and never creates an inbox/inject row.

use super::*;

const BACKEND_PK: &str = "backend-pubkey";

fn event(
    id: &str,
    kind: u32,
    author: &str,
    channel: &str,
    at: u64,
    body: &str,
    tags: &str,
) -> RelayEvent {
    RelayEvent {
        id: id.into(),
        kind,
        pubkey: author.into(),
        created_at: at,
        channel_h: channel.into(),
        d_tag: String::new(),
        content: body.into(),
        tags_json: tags.into(),
    }
}

#[test]
fn reaction_on_own_message_renders_once_then_is_silent() {
    let store = seed_store();
    let rec = session(&store);
    store.install_test_nmp_relay_delivery(
        TestRelayDelivery::new().profiles(seed_profiles()).events([
            event(
                "mymsg",
                9,
                SELF_PK,
                "root",
                100,
                "pushed the fix, tests green",
                "[]",
            ),
            event("rx1", 7, OTHER_PK, "root", 210, "👍", r#"[["e","mymsg"]]"#),
        ]),
    );

    // Turn whose cursor predates the reaction: it renders exactly once.
    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("reaction should render");
    assert_eq!(text.matches("<reactions>").count(), 1, "got: {text}");
    assert!(
        text.contains("@reviewer 👍 on your message \"pushed the fix, tests green\""),
        "got: {text}"
    );

    // Parity: the pure capture→assemble path renders identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false)).unwrap();
    let rendered = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(rendered, text);

    // A later turn whose cursor is past the reaction: nothing new → silent.
    let after = render_fabric_context(&store, input(Some(&rec), "root", 210, 300, false));
    assert!(after.is_none(), "reaction must not render again: {after:?}");

    // Forced, it collapses to the no-new-activity note (no reactions block).
    let forced = render_fabric_context(&store, input(Some(&rec), "root", 210, 300, true))
        .expect("forced who always renders");
    assert!(!forced.contains("<reactions>"), "got: {forced}");
    assert!(forced.contains("<no-new-activity"), "got: {forced}");

    // No inbox row was ever created for the reactor session — nothing to inject.
    assert!(store
        .peek_pending_for_pubkey(&rec.pubkey)
        .unwrap()
        .is_empty());
}

#[test]
fn backend_reactor_is_not_surfaced() {
    let store = seed_store();
    let mut profiles = seed_profiles();
    profiles.push(profile(BACKEND_PK, "daemon", "daemon", "", "laptop", true));
    let rec = session(&store);
    // A daemon 👁 receipt on my message must never appear in awareness.
    store.install_test_nmp_relay_delivery(TestRelayDelivery::new().profiles(profiles).events([
        event("mymsg", 9, SELF_PK, "root", 100, "shipped it", "[]"),
        event(
            "rx-eye",
            7,
            BACKEND_PK,
            "root",
            210,
            "👁",
            r#"[["e","mymsg"]]"#,
        ),
    ]));

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false));
    assert!(
        text.is_none(),
        "a backend-only reaction produces no awareness: {text:?}"
    );
}
