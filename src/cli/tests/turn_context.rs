use super::messaging::{format_envelope, EnvelopeView};
use crate::state::{Identity, RegisterSession, Session, Status, Store};
use crate::turn_context::{assemble_turn_check_context, assemble_turn_start_context};
use std::sync::Mutex;

const BACKEND: &str = "pk-backend";

/// Publish a relay_status (kind:30315) row — the single source awareness reads
/// for "who is doing what here", local and remote alike.
#[allow(clippy::too_many_arguments)]
fn pub_status(
    store: &Store,
    pubkey: &str,
    slug: &str,
    title: &str,
    activity: &str,
    busy: bool,
    updated_at: u64,
    now: u64,
) {
    store
        .upsert_status(&Status {
            pubkey: pubkey.to_string(),
            session_id: format!("sid-{slug}"),
            channel_h: "proj".to_string(),
            slug: slug.to_string(),
            title: title.to_string(),
            activity: activity.to_string(),
            busy,
            last_seen: updated_at,
            updated_at,
            expiration: now + 90,
        })
        .unwrap();
}

/// Materialize the `proj` channel + roster so awareness has fabric context.
fn seed_channel(store: &Store) {
    // Opaque id "proj" with a distinct human name "main" (production ids are random, never the name).
    store.upsert_channel("proj", "main", "", "", 1).unwrap();
    store
        .replace_channel_members("proj", &["pk-coder".to_string()], 1)
        .unwrap();
    store
        .upsert_profile_with_agent_slug("pk-coder", "coder", "coder", "coder", "laptop", false, 1)
        .unwrap();
}

/// A minimal alive [`Session`] for context assembly (the viewer).
fn test_session(id: &str) -> Session {
    Session {
        session_id: id.to_string(),
        agent_pubkey: "pk-coder".to_string(),
        agent_slug: "coder".to_string(),
        channel_h: "proj".to_string(),
        harness: "claude-code".to_string(),
        child_pid: None,
        transcript_path: None,
        alive: true,
        created_at: 1,
        last_seen: 1,
        working: false,
        turn_started_at: 0,
        last_distill_at: 0,
        seen_cursor: 0,
        title: String::new(),
        activity: String::new(),
        resume_id: String::new(),
        distill_fail_streak: 0,
        distill_notice_at: 0,
    }
}

/// turn_start returns None on a non-first turn with no inbox, chat, or peers.
#[test]
fn turn_start_context_returns_none_when_empty_non_first_turn() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let mut rec = test_session("sess-freeze-2");
    rec.seen_cursor = crate::util::now_secs();
    let m = Mutex::new(store);

    let ctx = assemble_turn_start_context(
        &m, &rec, BACKEND, "laptop", /* prev_turn_started_at */ 42,
    );
    assert!(
        ctx.is_none(),
        "turn_start with no inbox, non-first turn, no peers must return None; got: {ctx:?}"
    );
}

#[test]
fn first_turn_renders_awareness_snapshot_not_session_code() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let rec = test_session("sess-intro");
    let m = Mutex::new(store);

    let text = assemble_turn_start_context(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        text.contains("<tenex-edge>"),
        "first turn should render fabric awareness; got: {text:?}"
    );
    assert!(
        // Only the human name renders — the opaque id is never useful to an agent (addresses by name).
        text.contains("<channel name=\"#main\""),
        "awareness should name the channel; got: {text:?}"
    );
    let expected_code = crate::util::friendly_short_code("sess-intro");
    assert!(
        text.contains(&format!("You are @coder/{expected_code}, a coder agent.")),
        "awareness should identify this agent by agent/codename handle; got: {text:?}"
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
    let sid = store
        .register_session(&RegisterSession {
            harness: "codex".to_string(),
            external_id_kind: "session".to_string(),
            external_id: "sess-ordinal-native".to_string(),
            agent_pubkey: "pk-coder".to_string(),
            agent_slug: "coder".to_string(),
            channel_h: "proj".to_string(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now: 1,
        })
        .unwrap();
    store
        .upsert_identity(&Identity {
            pubkey: "pk-coder1".to_string(),
            agent_slug: "coder".to_string(),
            codename: "coder-vale-071".to_string(),
            session_id: sid.clone(),
            channel_h: "proj".to_string(),
            native_id: "sess-ordinal-native".to_string(),
            alive: true,
            created_at: 2,
        })
        .unwrap();
    let now = crate::util::now_secs();
    pub_status(
        &store,
        "pk-coder1",
        "coder-vale-071",
        "Session instance",
        "checking hook context",
        true,
        now,
        now,
    );
    let rec = store.get_session(&sid).unwrap().unwrap();
    let m = Mutex::new(store);

    let text = assemble_turn_start_context(&m, &rec, BACKEND, "laptop", 0)
        .expect("first-turn intro expected");
    assert!(
        text.contains("You are @coder/coder-vale-071, a coder agent."),
        "snapshot must render the bound session codename; got: {text:?}"
    );
    assert!(
        !text.contains("You are @coder,"),
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

    let text = assemble_turn_start_context(
        &m, &rec, BACKEND, "laptop", /* turn_end cleared this */ 0,
    )
    .expect("fresh chat past the cursor must surface");
    assert!(
        text.contains("<tenex-edge>") && text.contains("<chatter>"),
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
        text.contains("<recent-presence>"),
        "awareness update header expected; got: {text:?}"
    );
    assert!(
        text.contains("text=\"editing hooks.rs\""),
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

/// Ambient channel chat is delta-gated off the relay-event log: a row newer than
/// the cursor surfaces, an older one does not re-emit on the next tool call.
#[test]
fn turn_check_chat_shown_once_not_per_tool_call() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    // A kind:9 chat event in `proj`, created at 120 (after the cursor 50).
    store
        .insert_event(&crate::state::RelayEvent {
            id: "chat-new".to_string(),
            kind: 9,
            pubkey: "pk-chat".to_string(),
            created_at: 120,
            channel_h: "proj".to_string(),
            d_tag: String::new(),
            content: "ambient chatter".to_string(),
            tags_json: "[]".to_string(),
        })
        .unwrap();
    let m = Mutex::new(store);

    let text = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200)
        .expect("fresh chat past the cursor must surface");
    assert!(
        text.contains("<chatter>"),
        "chat should render inside the unified fabric update; got: {text:?}"
    );
    assert!(
        text.contains("ambient chatter"),
        "chat activity section expected; got: {text:?}"
    );
    assert!(
        !text.contains("Activity on #proj since your last check:"),
        "legacy ambient activity block must not render; got: {text:?}"
    );

    // Cursor advanced past the row (since=150 > 120): no re-emit.
    let text2 = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(150), 200);
    assert!(
        text2.is_none(),
        "already-shown chat must not repeat once the cursor passes it; got: {text2:?}"
    );
}

/// Direct deliveries come from the inbox ledger: a pending row surfaces at the
/// next hook even when the delta window is closed, then is marked delivered.
#[test]
fn turn_check_direct_mentions_surface_from_inbox() {
    let store = Store::open_memory().unwrap();
    seed_channel(&store);
    let newly = store
        .enqueue_inbox(
            "mention-1",
            "sess-me",
            "pk-chat",
            "proj",
            "please review this now",
            120,
        )
        .unwrap();
    assert!(newly, "first enqueue is newly parked");
    let m = Mutex::new(store);

    let ctx = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", None, 200)
        .expect("direct mention must surface at the next available hook");
    assert!(ctx.contains("please review this now"), "got: {ctx:?}");

    // Drained → marked delivered → not handled-as-pending again.
    let s = m.lock().unwrap();
    assert!(
        s.peek_pending_for_session("sess-me").unwrap().is_empty(),
        "delivered mention must not remain pending"
    );
    assert!(s.is_event_handled("mention-1", "sess-me").unwrap());
}

// ── envelope rendering (pure; unchanged by the persistence rewrite) ───────────

fn view<'a>() -> EnvelopeView<'a> {
    EnvelopeView {
        from_slug: "codex",
        from_session: "sender-session-id",
        host: "",
        self_host: "my-box",
        subject: "NIP-29 group creation failing",
        branch: "features/oauth",
        commit: "a1b2c3d",
        dirty: 0,
        id: "01234567",
        sent_at: 1_000,
        now: 1_180, // +3 min
        body: "can you take a look?",
    }
}

#[test]
fn envelope_has_email_like_headers_then_body() {
    let out = format_envelope(&view());
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[0], "From: codex@my-box");
    assert!(lines[1].starts_with("Date: ") && lines[1].ends_with("(3 min ago)"));
    assert_eq!(lines[2], "Subject: NIP-29 group creation failing");
    assert_eq!(lines[3], "Branch: features/oauth (a1b2c3d)");
    assert_eq!(lines[4], "ID: 01234567");
    assert_eq!(lines[5], "--");
    assert_eq!(lines[6], "can you take a look?");
}

#[test]
fn dirty_count_and_remote_host_annotate() {
    let mut v = view();
    v.dirty = 1;
    v.host = "prodBackend";
    let out = format_envelope(&v);
    assert!(out.contains("From: codex@prodBackend"));
    assert!(out.contains("Branch: features/oauth (a1b2c3d) [1 file dirty]"));
    v.dirty = 3;
    assert!(format_envelope(&v).contains("[3 files dirty]"));
}

#[test]
fn subject_and_branch_lines_omitted_when_empty() {
    let mut v = view();
    v.subject = "";
    v.branch = "";
    let out = format_envelope(&v);
    assert!(!out.contains("Subject:"));
    assert!(!out.contains("Branch:"));
    assert!(!out.contains("remote:"));
}
