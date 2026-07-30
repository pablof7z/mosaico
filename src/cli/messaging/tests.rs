use super::*;

#[test]
fn channel_read_row_prints_truncation_recovery_command() {
    let item = serde_json::json!({
        "event_id": "event-123",
        "from_pubkey": "pubkey-1",
        "from_slug": "writer",
        "host": "laptop",
        "body": "word0 word1...",
        "truncated": true,
        "created_at": 1_000,
    });
    let text = render_channel_read_row(&item, false);
    assert!(text.contains("<writer@laptop> word0 word1..."));
    assert!(text.contains("mosaico channel read --id event-123"));
}

#[test]
fn channel_read_row_renders_hostless_sender_bare_not_with_question_mark() {
    let item = serde_json::json!({
        "event_id": "event-124",
        "from_pubkey": "pubkey-2",
        "from_slug": "Pablo",
        "host": "",
        "body": "hi",
        "truncated": false,
        "created_at": 1_000,
    });
    let text = render_channel_read_row(&item, false);
    assert!(text.starts_with("<@Pablo> hi"), "got: {text}");
    assert!(!text.contains('?'), "got: {text}");
}

#[test]
fn channel_read_row_renders_materialized_attachment_directory() {
    let item = serde_json::json!({
        "event_id": "event-files",
        "from_pubkey": "pubkey-3",
        "from_slug": "writer",
        "host": "laptop",
        "body": "the files are ready",
        "attachment_dir": "/home/alice/.mosaico/tmp/attachments/abcdef",
        "created_at": 1_000,
    });

    let text = render_channel_read_row(&item, false);
    assert!(text.contains("\n[attachments: /home/alice/.mosaico/tmp/attachments/abcdef]"));
}
