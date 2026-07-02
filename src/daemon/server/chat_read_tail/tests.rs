use super::*;
use crate::util::CHAT_RENDER_WORD_LIMIT;

fn row(body: String) -> RelayEvent {
    RelayEvent {
        id: "event-1".into(),
        kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
        pubkey: "pubkey-1".into(),
        created_at: 123,
        channel_h: "channel-1".into(),
        d_tag: String::new(),
        content: body,
        tags_json: "[]".into(),
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
    let json = chat_log_row_to_json(&row(message(CHAT_RENDER_WORD_LIMIT + 1)), "writer", true);
    assert_eq!(json["event_id"], "event-1");
    assert_eq!(json["full_event_id"], "event-1");
    assert_eq!(json["truncated"], true);
    assert!(json["body"].as_str().unwrap().ends_with("..."));
}

#[test]
fn chat_log_json_keeps_exact_id_reads_full() {
    let body = message(CHAT_RENDER_WORD_LIMIT + 1);
    let json = chat_log_row_to_json(&row(body.clone()), "writer", false);
    assert_eq!(json["truncated"], false);
    assert_eq!(json["body"], body);
}
