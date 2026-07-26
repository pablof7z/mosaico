use crate::state::{RegisterSession, Store};
use crate::turn_context::{assemble_turn_check_context, render_turn_start_text_for_test};
use std::sync::Mutex;

#[path = "turn_context/delivery.rs"]
mod delivery;
#[path = "turn_context/envelope.rs"]
mod envelope;
#[path = "turn_context/fixtures.rs"]
mod fixtures;
use fixtures::{pub_status, seed_channel, test_session, BACKEND};

/// A quiet headed turn emits no context merely to announce ordinary visibility.
#[test]
fn quiet_non_first_headed_turn_emits_no_context() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let mut rec = test_session("sess-freeze-2");
    rec.seen_cursor = crate::util::now_secs();
    let m = Mutex::new(store);

    let ctx = render_turn_start_text_for_test(
        &m, &rec, BACKEND, "laptop", /* prev_turn_started_at */ 42,
    );
    assert!(ctx.is_none(), "headed mode should be silent; got: {ctx:?}");
}

#[test]
fn first_turn_renders_awareness_snapshot_not_session_code() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-intro");
    let m = Mutex::new(store);
    let text = render_turn_start_text_for_test(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        text.contains("<mosaico>"),
        "first turn should render fabric awareness; got: {text:?}"
    );
    assert!(
        text.contains("<channel name=\"/proj\"") && !text.contains("<workspace"),
        "awareness should use the public root-channel path without a workspace wrapper; got: {text:?}"
    );
    assert!(
        text.contains("<self name=\"@coder\" host=\"laptop\""),
        "awareness should not derive a handle from the session id; got: {text:?}"
    );
    assert!(
        !text.contains("[session"),
        "intro must not expose a session code; got: {text:?}"
    );
}

#[test]
fn first_turn_snapshot_uses_bound_instance_identity() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    store
        .replace_channel_members("proj", &["pk-coder1".to_string()], 2)
        .unwrap();
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: "pk-coder1".to_string(),
            observed_harness: "codex".to_string(),
            agent_slug: "coder".to_string(),
            launch_channel_h: "proj".to_string(),
            work_root: "proj".to_string(),
            child_pid: None,
            now: 1,
        })
        .unwrap();
    store
        .bind_session_signer("pk-coder1", "test-signer-salt")
        .unwrap();
    store
        .allocate_custom_handle("pk-coder1", "coder", "willow-vale-071", 2)
        .unwrap();
    let now = crate::util::now_secs();
    pub_status(
        &store,
        "pk-coder1",
        "willow-vale-071-coder",
        "Session instance",
        "checking hook context",
        true,
        now,
        now,
    );
    let rec = store.get_session("pk-coder1").unwrap().unwrap();
    let m = Mutex::new(store);

    let text = render_turn_start_text_for_test(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        text.contains("<self name=\"@willow-vale-071-coder\" host=\"laptop\""),
        "snapshot must render the bound session codename; got: {text:?}"
    );
    assert!(
        !text.contains("<self name=\"@coder\""),
        "bare agent slug must not override the bound session handle; got: {text:?}"
    );
}

#[test]
fn ended_turn_with_cursor_uses_delta_not_snapshot() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    store
        .insert_event(&crate::state::RelayEvent {
            id: "chat-after-cursor".to_string(),
            kind: 9,
            pubkey: "pk-chat".to_string(),
            created_at: 160,
            channel_h: "proj".to_string(),
            d_tag: String::new(),
            content: "new message after prior turn".to_string(),
            tags_json: "[]".to_string(),
        })
        .unwrap();
    let mut rec = test_session("sess-ended-turn");
    rec.seen_cursor = 150;
    let m = Mutex::new(store);

    let text = render_turn_start_text_for_test(
        &m, &rec, BACKEND, "laptop", /* turn_end cleared this */ 0,
    )
    .expect("fresh chat past the cursor must surface");
    assert!(
        text.contains("<mosaico>") && text.contains("<chatter>"),
        "ended turn should render a delta, got: {text:?}"
    );
    assert!(
        !text.contains("<members>"),
        "static fabric snapshot must not repeat after the cursor advanced; got: {text:?}"
    );
    assert!(
        !text.contains("since you joined"),
        "post-first-turn chat must not be labelled as join-time context; got: {text:?}"
    );
}

/// A first turn with no PTY locator for the session (the daemon has no
/// live PTY endpoint to inject into) carries the not-PTY-wrapped warning, so
/// the agent learns idle mentions won't reach it until its next turn
/// (`src/reconcile/delivery/mod.rs` returns `DeferNoEndpoint` and drops them).
#[test]
fn first_turn_warns_when_session_has_no_live_pty_endpoint() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-no-pty");
    let m = Mutex::new(store);

    let text = render_turn_start_text_for_test(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        text.contains("This session cannot be steered while idle."),
        "expected the not-PTY-wrapped warning; got: {text:?}"
    );
    assert!(!text.contains("keep taking turns"), "got: {text:?}");
    assert!(!text.contains("pty-wrap-me"), "got: {text:?}");
}

/// The same first turn with a live PTY locator omits the warning: the
/// daemon has a real endpoint it can inject idle mentions into.
#[test]
fn first_turn_omits_pty_warning_when_session_has_a_live_endpoint() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let mut rec = test_session("sess-with-pty");
    rec.admitted_transport = "pty".into();

    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("live.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    store
        .put_session_locator(
            "claude-code",
            crate::state::LOCATOR_PTY,
            socket_path.to_str().unwrap(),
            &rec.pubkey,
            1,
        )
        .unwrap();
    let m = Mutex::new(store);

    let text = render_turn_start_text_for_test(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        !text.contains("This session cannot be steered while idle."),
        "a live PTY locator must suppress the not-PTY-wrapped warning; got: {text:?}"
    );
}

/// turn_check returns None when there is no inbox and delta_since=None.
#[test]
fn turn_check_context_returns_none_when_nothing_due() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let m = Mutex::new(store);
    let ctx = assemble_turn_check_context(&m, &test_session("sess-no-rows"), "laptop", None, 200);
    assert!(
        ctx.is_none(),
        "turn_check with no inbox, no delta → None; got: {ctx:?}"
    );
}

/// Mid-turn delta: a sibling's relay_status change in the same channel surfaces
/// with its activity line; the viewer's own status (same pubkey) is excluded.
#[test]
fn turn_check_delta_shows_siblings_with_activity_excludes_self() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    store
        .replace_channel_members("proj", &["pk-coder".into(), "pk-sib".into()], 2)
        .unwrap();
    // Sibling changed after the cursor (50) and is still live at now=200.
    pub_status(
        &store,
        "pk-sib",
        "sib",
        "Refactor PTY hosting",
        "editing hooks.rs",
        true,
        180,
        200,
    );
    // The viewer's own status also changed — must NOT echo back.
    pub_status(
        &store,
        "pk-coder",
        "coder",
        "My own work",
        "typing",
        true,
        180,
        200,
    );
    let m = Mutex::new(store);

    let text = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200)
        .expect("delta block expected when a sibling changed");
    assert!(
        text.contains("<members>") && !text.contains("<recent-presence>"),
        "presence deltas should use the same members block as full state; got: {text:?}"
    );
    assert!(
        text.contains("status=\"editing hooks.rs\""),
        "sibling activity expected as a member work line; got: {text:?}"
    );
    assert!(
        !text.contains("My own work"),
        "viewer's own status must be excluded; got: {text:?}"
    );
}

/// `delta_since = None` (rate-limited / not mid-turn) suppresses the sibling
/// delta entirely, even when a sibling just changed.
#[test]
fn turn_check_delta_suppressed_when_not_due() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    pub_status(
        &store,
        "pk-sib",
        "sib",
        "Refactor PTY hosting",
        "",
        true,
        180,
        200,
    );
    let m = Mutex::new(store);

    let ctx = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", None, 200);
    assert!(
        ctx.is_none(),
        "no delta and no inbox → None when not due; got: {ctx:?}"
    );
}
