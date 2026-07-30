use crate::state::{RegisterSession, RelayEvent, Store};
use std::sync::Mutex;

mod advisory;
mod unscoped;

// Two distinct (fake) pubkeys used throughout — long enough for SQLite but
// not real Nostr pubkeys (unit tests do not sign or verify).
const SELF_PK: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_PK: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn register(store: &Store, pk: &str, channel: &str, now: u64) -> String {
    if store.get_channel(channel).unwrap().is_none() {
        store.upsert_channel(channel, channel, "", "", now).unwrap();
    }
    store
        .reserve_hook_session_for_test(&RegisterSession {
            pubkey: pk.to_string(),
            observed_harness: "codex".into(),
            agent_slug: "test-agent".into(),
            launch_channel_h: channel.to_string(),
            work_root: channel.to_string(),
            child_pid: None,
            now,
        })
        .unwrap();
    pk.to_string()
}

fn materialize_channel(store: &Store, channel: &str) {
    store.upsert_channel(channel, channel, "", "", 1).unwrap();
    store
        .replace_channel_members(channel, &[SELF_PK.to_string()], 1)
        .unwrap();
    store
        .upsert_profile(SELF_PK, "test-agent", "test-agent", "", false, 1)
        .unwrap();
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

fn insert_mention(store: &Store, id: &str, channel: &str, created_at: u64, body: &str) {
    store
        .insert_event(&RelayEvent {
            id: id.to_string(),
            kind: 9,
            pubkey: OTHER_PK.to_string(),
            created_at,
            channel_h: channel.to_string(),
            d_tag: String::new(),
            content: body.to_string(),
            tags_json: format!("[[\"p\",\"{SELF_PK}\"]]"),
        })
        .unwrap();
    store
        .enqueue_inbox(id, SELF_PK, OTHER_PK, channel, body, created_at)
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
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("3 messages in a prior 1 min activity cluster")
            && ctx.contains("explicit channel read"),
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
        materialize_channel(&s, ch);
        let id = register(&s, SELF_PK, ch, now); // session starts inside the recent window
        s.get_session(&id).unwrap().unwrap()
    };
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, now + 10, "post-join-message");
    }
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("post-join-message"),
        "post-join chat should appear in ambient; got:\n{ctx}"
    );
    assert!(
        !ctx.contains("before you joined"),
        "no pre-join notice when channel was empty at join time; got:\n{ctx}"
    );
}

/// Even an otherwise empty first turn carries the one-shot skill reminder.
#[test]
fn first_turn_empty_channel_still_loads_mosaico_skill() {
    let m = Mutex::new(Store::open_memory().unwrap());
    let ch = "ch-empty";
    let rec = {
        let s = m.lock().unwrap();
        let id = register(&s, SELF_PK, ch, 100);
        s.get_session(&id).unwrap().unwrap()
    };
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0)
        .expect("first hook must inject the skill reminder");
    assert!(
        ctx.contains("You must load the `mosaico` skill right away, before continuing."),
        "first hook should require immediate Mosaico skill loading; got:\n{ctx}"
    );
    assert!(
        !ctx.contains("<message "),
        "empty channel should produce no message context; got:\n{ctx}"
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
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("1 message in a prior 1 min activity cluster")
            && ctx.contains("explicit channel read"),
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
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    // Event before session start — surfaces as pre-join notice on first turn.
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, 50, "pre-join-event");
    }
    // First turn: consumes pre-join notice.
    {
        let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
        let _ = super::render_turn_start_text_for_test(&m, &rec, "", "", 0);
    }
    // Manually peg the cursor at t=150 so the second turn only sees t>150.
    let store = m.lock().unwrap();
    let expected = store.get_session(&sid).unwrap().unwrap().seen_cursor;
    assert!(store
        .advance_cursor_if_current(&sid, expected, 150)
        .unwrap());
    drop(store);
    // Event after the cursor — should appear in the second turn.
    {
        let s = m.lock().unwrap();
        insert_chat(&s, ch, OTHER_PK, 160, "second-turn-event");
    }
    let rec2 = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    assert_eq!(rec2.seen_cursor, 150, "cursor must be 150 for this test");
    let ctx2 = super::render_turn_start_text_for_test(&m, &rec2, "", "", 0).unwrap_or_default();
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
    assert!(
        !ctx2.contains("load the `mosaico` skill"),
        "skill reminder must not repeat after the first hook; got:\n{ctx2}"
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
        materialize_channel(&s, ch);
        register(&s, SELF_PK, ch, 100)
    };
    {
        let s = m.lock().unwrap();
        insert_mention(&s, "ev-mention-1", ch, 110, "hey do the thing");
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
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
        materialize_channel(&s, ch);
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
        insert_mention(&s, "ev-dm-1", ch, now + 15, "start working on X");
    }
    let rec = m.lock().unwrap().get_session(&sid).unwrap().unwrap();
    let ctx = super::render_turn_start_text_for_test(&m, &rec, "", "", 0).unwrap_or_default();
    assert!(
        ctx.contains("start working on X"),
        "direct mention must appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("ambient-background-chat"),
        "post-join ambient chat must also appear; got:\n{ctx}"
    );
    assert!(
        ctx.contains("<chatter>")
            && ctx.contains(
                "Need a follow-up? Read `skills/mosaico/references/coordination-guide.md`."
            ),
        "ambient chat and mention must render in the fabric context; got:\n{ctx}"
    );
}
