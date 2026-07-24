use super::*;
use crate::state::{Message, Store};
use crate::util::CHAT_RENDER_WORD_LIMIT;

fn row(body: String) -> Message {
    Message {
        message_id: "event-1".into(),
        thread_id: "channel-1".into(),
        channel_h: "channel-1".into(),
        author_pubkey: "pubkey-1".into(),
        body,
        created_at: 123,
        direction: "inbound".into(),
        sync_state: "accepted".into(),
        native_event_id: Some("event-1".into()),
        error: None,
    }
}

fn message(words: usize) -> String {
    (0..words)
        .map(|i| format!("word{i}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn chat_log_json_truncates_regular_reads() {
    let json = chat_log_row_to_json(
        &row(message(CHAT_RENDER_WORD_LIMIT + 1)),
        "writer",
        "laptop",
        true,
    );
    assert_eq!(json["event_id"], "event-1");
    assert_eq!(json["full_event_id"], "event-1");
    assert_eq!(json["truncated"], true);
    assert!(json["body"].as_str().unwrap().ends_with("..."));
    assert!(json.get("from_session").is_none());
    assert!(json.get("mentioned_session").is_none());
}

#[test]
fn chat_log_json_keeps_exact_id_reads_full() {
    let body = message(CHAT_RENDER_WORD_LIMIT + 1);
    let json = chat_log_row_to_json(&row(body.clone()), "writer", "laptop", false);
    assert_eq!(json["truncated"], false);
    assert_eq!(json["body"], body);
}

#[test]
fn root_channel_read_backfill_and_live_scopes_include_nested_descendants() {
    let store = crate::state::Store::open_memory().unwrap();
    store.upsert_channel("root", "channel", "", "", 1).unwrap();
    store.upsert_channel("task", "Task", "", "root", 2).unwrap();
    store.upsert_channel("deep", "Deep", "", "task", 3).unwrap();
    store.upsert_channel("leaf", "Leaf", "", "deep", 4).unwrap();
    store.upsert_channel("other", "Other", "", "", 5).unwrap();

    assert_eq!(
        channel_read_scopes_for_store(&store, "root"),
        vec![
            "deep".to_string(),
            "leaf".to_string(),
            "root".to_string(),
            "task".to_string()
        ]
    );
    assert_eq!(
        channel_read_scopes_for_store(&store, "task"),
        vec!["task".to_string()]
    );
    assert_eq!(
        channel_read_scopes_for_store(&store, "unknown"),
        vec!["unknown".to_string()]
    );
}

#[test]
fn channel_read_live_lag_is_terminal_stream_error() {
    let resp = stream_lag_error(42, "channel read --live", 3);

    let err = resp.error.expect("lag response is an error");
    assert_eq!(err.code, "stream_lagged");
    assert!(err
        .message
        .contains("channel read --live dropped 3 live event"));
    assert!(err.message.contains("reconnect"));
}

#[test]
fn tail_lag_is_terminal_stream_error() {
    let resp = stream_lag_error(7, "tail", 11);

    let err = resp.error.expect("lag response is an error");
    assert_eq!(err.code, "stream_lagged");
    assert!(err.message.contains("tail dropped 11 live event"));
}

// ── mention resolution + backend-traffic + whitelisted-host (regression) ──────

const TARGET_PK: &str = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";
const BACKEND_PK: &str = "9aa6883eee2f1ce43053a1eec2c1c8b1c712cbb3c77ec346d9f091982a50b461";
const HUMAN_PK: &str = "b1c712cbb3c77ec346d9f091982a50b461379e863e8357163b5bce5d2688dc4f";

#[tokio::test]
async fn chat_row_to_json_rewrites_nostr_mentions_in_body() {
    use nostr::{PublicKey, ToBech32};

    let state = DaemonState::new_for_test().await;
    state.with_store(|s| {
        s.upsert_profile(TARGET_PK, "target@laptop", "target", "laptop", false, 1)
            .unwrap();
    });
    let npub = PublicKey::from_hex(TARGET_PK).unwrap().to_bech32().unwrap();
    let msg = row(format!("please ask nostr:{npub} for review"));

    let json = chat_row_to_json(&state, &msg, false);
    assert_eq!(json["body"], "please ask @target@laptop for review");
}

#[test]
fn is_backend_row_true_when_author_is_flagged_backend() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile(BACKEND_PK, "laptop (mosaico)", "hub", "laptop", true, 1)
        .unwrap();
    let mut msg = row("mgmt ok: 14 agent(s) on laptop".to_string());
    msg.author_pubkey = BACKEND_PK.to_string();

    assert!(is_backend_row(&store, "", &msg));
}

#[test]
fn is_backend_row_true_when_recipient_is_flagged_backend() {
    let store = Store::open_memory().unwrap();
    store
        .upsert_profile(BACKEND_PK, "laptop (mosaico)", "hub", "laptop", true, 1)
        .unwrap();
    let msg = row("list agents".to_string());
    store
        .add_message_recipient(&msg.message_id, BACKEND_PK, None)
        .unwrap();

    assert!(is_backend_row(&store, "", &msg));
}

#[test]
fn is_backend_row_false_for_ordinary_chat() {
    let store = Store::open_memory().unwrap();
    let msg = row("hi Pablo".to_string());

    assert!(!is_backend_row(&store, "", &msg));
}

#[tokio::test]
async fn chat_row_refs_renders_whitelisted_human_without_host() {
    let state = DaemonState::new_for_test_with_whitelisted(vec![HUMAN_PK.to_string()]).await;
    let mut msg = row("hi".to_string());
    msg.author_pubkey = HUMAN_PK.to_string();

    let (_, host) = chat_row_refs(&state, &msg);
    assert_eq!(
        host, "",
        "whitelisted human must render with no host, not `?`"
    );
}
