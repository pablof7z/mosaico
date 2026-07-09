use super::*;
use crate::state::Message;
use crate::util::CHAT_RENDER_WORD_LIMIT;

fn row(body: String) -> Message {
    Message {
        message_id: "event-1".into(),
        thread_id: "channel-1".into(),
        channel_h: "channel-1".into(),
        author_pubkey: "pubkey-1".into(),
        author_session: Some("sender-session".into()),
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
        Some("target-session"),
        true,
    );
    assert_eq!(json["event_id"], "event-1");
    assert_eq!(json["full_event_id"], "event-1");
    assert_eq!(json["truncated"], true);
    assert!(json["body"].as_str().unwrap().ends_with("..."));
    assert_eq!(json["from_session"], "sender-session");
    assert_eq!(json["mentioned_session"], "target-session");
}

#[test]
fn chat_log_json_keeps_exact_id_reads_full() {
    let body = message(CHAT_RENDER_WORD_LIMIT + 1);
    let json = chat_log_row_to_json(&row(body.clone()), "writer", "laptop", None, false);
    assert_eq!(json["truncated"], false);
    assert_eq!(json["body"], body);
}

#[test]
fn root_chat_read_backfill_and_live_scopes_include_nested_descendants() {
    let store = crate::state::Store::open_memory().unwrap();
    store.upsert_channel("root", "channel", "", "", 1).unwrap();
    store.upsert_channel("task", "Task", "", "root", 2).unwrap();
    store.upsert_channel("deep", "Deep", "", "task", 3).unwrap();
    store.upsert_channel("leaf", "Leaf", "", "deep", 4).unwrap();
    store.upsert_channel("other", "Other", "", "", 5).unwrap();

    assert_eq!(
        chat_read_scopes_for_store(&store, "root"),
        vec![
            "deep".to_string(),
            "leaf".to_string(),
            "root".to_string(),
            "task".to_string()
        ]
    );
    assert_eq!(
        chat_read_scopes_for_store(&store, "task"),
        vec!["task".to_string()]
    );
    assert_eq!(
        chat_read_scopes_for_store(&store, "unknown"),
        vec!["unknown".to_string()]
    );
}

#[test]
fn chat_read_live_lag_is_terminal_stream_error() {
    let resp = stream_lag_error(42, "chat read --live", 3);

    let err = resp.error.expect("lag response is an error");
    assert_eq!(err.code, "stream_lagged");
    assert!(err
        .message
        .contains("chat read --live dropped 3 live event"));
    assert!(err.message.contains("reconnect"));
}

#[test]
fn tail_lag_is_terminal_stream_error() {
    let resp = stream_lag_error(7, "tail", 11);

    let err = resp.error.expect("lag response is an error");
    assert_eq!(err.code, "stream_lagged");
    assert!(err.message.contains("tail dropped 11 live event"));
}
