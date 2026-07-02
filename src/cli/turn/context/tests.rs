use crate::state::{RegisterSession, RelayEvent, Store};
use std::sync::Mutex;

// Two distinct (fake) pubkeys used throughout — long enough for SQLite but
// not real Nostr pubkeys (unit tests do not sign or verify).
const SELF_PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_PK: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn register(store: &Store, pk: &str, channel: &str, now: u64) -> String {
    store
        .register_session(&RegisterSession {
            harness: "test".into(),
            external_id_kind: "test".into(),
            external_id: format!("{pk}-{now}"),
            agent_pubkey: pk.to_string(),
            agent_slug: "test-agent".into(),
            channel_h: channel.to_string(),
            child_pid: None,
            transcript_path: None,
            resume_id: String::new(),
            now,
        })
        .unwrap()
}

fn insert_chat(store: &Store, channel: &str, pubkey: &str, created_at: u64, body: &str) {
    store
        .insert_event(&RelayEvent {
            id: format!("ev-{pubkey}-{created_at}"),
            kind: 9,
            pubkey: pubkey.to_string(),
            created_at,
            channel_h: channel.to_string(),
            d_tag: String::new(),
            content: body.to_string(),
            tags_json: "[]".to_string(),
        })
        .unwrap();
}

/// Pre-join history (messages before session.created_at) is announced as a
/// compact count, never dumped inline.
#[test]
fn first_turn_pre_join_history_compact_notice() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-notice";
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, 10, "ancient msg 1");
        insert_chat(&s, ch, OTHER_PK, 20, "ancient msg 2");
        insert_chat(&s, ch, OTHER_PK, 30, "ancient msg 3");
    }
    let rec = {
        let s = m.lock().unwrap();
        let id = register(&s, SELF_PK, ch, 100); // session starts at t=100
        s.get_session(&id).unwrap().unwrap()
    };
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("3 message(s)") && ctx.contains("before you joined"),
        "pre-join history should be announced as a compact count; got:\n{ctx}"
    );
    assert!(
        !ctx.contains("ancient msg 1"),
        "pre-join message content must NOT be dumped inline; got:\n{ctx}"
    );
}

/// Messages that arrive between session start and the first turn DO appear
/// as ambient context (post-join window).
#[test]
fn first_turn_post_join_chat_shown_as_ambient() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-postjoin";
    let now = crate::util::now_secs().saturating_sub(100);
    let rec = {
        let s = m.lock().unwrap();
        let id = register(&s, SELF_PK, ch, now); // session starts inside the recent window
        s.get_session(&id).unwrap().unwrap()
    };
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, now + 10, "post-join-message");
    }
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("post-join-message"),
        "post-join chat should appear in ambient; got:\n{ctx}"
    );
    assert!(
        !ctx.contains("before you joined"),
        "no pre-join notice when channel was empty at join time; got:\n{ctx}"
    );
}

/// Channel is completely empty when the session starts and stays empty —
/// first turn returns no context.
#[test]
fn first_turn_empty_channel_no_context() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-empty";
    let rec = {
        let s = m.lock().unwrap();
        let id = register(&s, SELF_PK, ch, 100);
        s.get_session(&id).unwrap().unwrap()
    };
    // No events at all — should return None (no context blocks).
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0);
    assert!(
        ctx.is_none()
            || ctx
                .as_deref()
                .map(|s| !s.contains("message") || s.contains("not a member"))
                .unwrap_or(true),
        "empty channel should produce no message context; got:\n{ctx:?}"
    );
}

/// Self-authored messages that predate the session also count toward the
/// pre-join notice (total channel history, regardless of author).
#[test]
fn first_turn_self_authored_pre_join_events_count_for_notice() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-self-pre";
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, SELF_PK, 5, "self-earlier-message");
    }
    let rec = {
        let s = m.lock().unwrap();
        let id = register(&s, SELF_PK, ch, 100);
        s.get_session(&id).unwrap().unwrap()
    };
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("1 message(s)") && ctx.contains("before you joined"),
        "self-authored pre-join messages should count toward notice; got:\n{ctx}"
    );
}

/// Second turn uses the seen_cursor (not session.created_at) for the
/// ambient window, so messages shown in the first turn don't re-appear.
#[test]
fn second_turn_ambient_gates_on_seen_cursor() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-cursor";
    let sid = {
        let s = m.lock().unwrap();
        register(&s, SELF_PK, ch, 100)
    };
    // Event before session start — surfaces as pre-join notice on first turn.
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, 50, "pre-join-event");
    }
    // First turn: consumes pre-join notice; seen_cursor → now_secs().
    {
        let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
        let _ = super::assemble_turn_start_context(&m, &rec, "", "", 0);
    }
    // Manually peg the cursor at t=150 so the second turn only sees t>150.
    m.lock().unwrap().set_seen_cursor(&sid, 150).unwrap();
    // Event after the cursor — should appear in the second turn.
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, 160, "second-turn-event");
    }
    let rec2 = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    assert_eq!(rec2.seen_cursor, 150, "cursor must be 150 for this test");
    let ctx2 = super::assemble_turn_start_context(&m, &rec2, "", "", 0).unwrap_or_default();
    assert!(
        ctx2.contains("second-turn-event"),
        "second turn must show messages since cursor; got:\n{ctx2}"
    );
    assert!(
        !ctx2.contains("pre-join-event"),
        "pre-cursor message must not appear in the fabric delta; got:\n{ctx2}"
    );
    assert!(
        !ctx2.contains("before you joined"),
        "pre-join notice must not appear on second turn; got:\n{ctx2}"
    );
}

/// An inbox mention (p-tagged, enqueued via enqueue_inbox) appears in the
/// turn context as a direct-mention block.
#[test]
fn inbox_mention_surfaces_in_turn_context() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-mention";
    let sid = {
        let s = m.lock().unwrap();
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        s.enqueue_inbox("ev-mention-1", &sid, OTHER_PK, ch, "hey do the thing", 110)
            .unwrap();
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("hey do the thing"),
        "inbox mention must appear in turn context; got:\n{ctx}"
    );
}

/// Ambient channel chat (not in inbox) is shown alongside an inbox mention in
/// the same structured fabric context.
#[test]
fn ambient_and_mention_both_in_first_turn_context() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-dual";
    let now = crate::util::now_secs().saturating_sub(100);
    let sid = {
        let s = m.lock().unwrap();
        register(&s, SELF_PK, ch, now)
    };
    // Ambient (non-mention) message arriving after session start.
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, now + 10, "ambient-background-chat");
    }
    // Direct mention in inbox.
    {
        let s = m.lock().unwrap();
        s.enqueue_inbox(
            "ev-dm-1",
            &sid,
            OTHER_PK,
            ch,
            "start working on X",
            now + 15,
        )
        .unwrap();
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::assemble_turn_start_context(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("start working on X"),
        "direct mention must appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("ambient-background-chat"),
        "post-join ambient chat must also appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("<chatter>") && ctx.contains("mention=\"true\""),
        "ambient chat and mention must render in the fabric context; got:\n{ctx}"
    );
    assert!(
        !ctx.contains("Activity on #"),
        "legacy ambient activity block must not render; got:\n{ctx}"
    );
}
