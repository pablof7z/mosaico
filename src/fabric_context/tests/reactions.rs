//! Turn-start reaction awareness: a materialized kind:7 on the caller's own
//! message renders exactly once, is cursor-gated so it never repeats, ignores
//! backend (daemon 👁 receipt) reactors, and never creates an inbox/inject row.

use super::*;
use crate::state::RecordMessage;

const BACKEND_PK: &str = "backend-pubkey";

fn record_self_message(store: &Store, id: &str, channel: &str, at: u64, body: &str) {
    store
        .record_message(&RecordMessage {
            message_id: id.into(),
            thread_id: channel.into(),
            channel_h: channel.into(),
            author_pubkey: SELF_PK.into(),
            body: body.into(),
            created_at: at,
            direction: "outbound".into(),
            sync_state: "accepted".into(),
            native_event_id: Some(id.into()),
            error: None,
        })
        .unwrap();
}

#[test]
fn reaction_on_own_message_renders_once_then_is_silent() {
    let store = seed_store();
    let rec = session(&store);
    record_self_message(&store, "mymsg", "root", 100, "pushed the fix, tests green");
    store
        .upsert_reaction("rx1", "mymsg", "root", OTHER_PK, "👍", 210)
        .unwrap();

    // Turn whose cursor predates the reaction: it renders exactly once.
    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false))
        .expect("reaction should render");
    assert_eq!(text.matches("<reactions>").count(), 1, "got: {text}");
    assert!(
        text.contains("@reviewer 👍 on your message \"pushed the fix, tests green\""),
        "got: {text}"
    );

    // Parity: the pure capture→assemble path renders identically.
    let captured = capture_inputs(&store, &input(Some(&rec), "root", 200, 300, false));
    let trellis = render_view_text(&assemble::assemble_view(&captured, 200, 300));
    assert_eq!(trellis, text);

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
    store
        .upsert_profile(BACKEND_PK, "daemon", "daemon", "laptop", true, 1)
        .unwrap();
    let rec = session(&store);
    record_self_message(&store, "mymsg", "root", 100, "shipped it");
    // A daemon 👁 receipt on my message must never appear in awareness.
    store
        .upsert_reaction("rx-eye", "mymsg", "root", BACKEND_PK, "👁", 210)
        .unwrap();

    let text = render_fabric_context(&store, input(Some(&rec), "root", 200, 300, false));
    assert!(
        text.is_none(),
        "a backend-only reaction produces no awareness: {text:?}"
    );
}
