use super::*;

#[test]
fn renderer_preserves_daemon_owned_reminders() {
    let result = serde_json::json!({
        "recipient_reminders": [
            "Reminder: @one is suspended and will receive this message after manual resumption.",
            "Reminder: @two is suspended and will receive this message after manual resumption."
        ],
        "coaching": []
    });

    assert_eq!(
        recipient_reminders(&result).unwrap(),
        vec![
            "Reminder: @one is suspended and will receive this message after manual resumption.",
            "Reminder: @two is suspended and will receive this message after manual resumption."
        ]
    );
}

#[test]
fn renderer_rejects_an_incomplete_result_contract() {
    let error = recipient_reminders(&serde_json::json!({})).unwrap_err();
    assert!(error
        .to_string()
        .contains("daemon response missing recipient_reminders"));
}

#[test]
fn renderer_preserves_summaries_and_builds_an_explicit_correction_command() {
    let result = serde_json::json!({
        "event_id": "abcdef1234567890",
        "channel": "#mosaico/reviews",
        "coaching": [{
            "level": "warn",
            "code": "untagged_agent_prefix",
            "summary": "WARN: published ambient chat",
            "matched_agent": "drift-codex"
        }]
    });

    assert_eq!(
        send_coaching_lines(&result).unwrap(),
        vec![
            "WARN: published ambient chat",
            "To tag that agent now, run: `mosaico channel send --channel \
             '#mosaico/reviews' --tag 'drift-codex' --message \
             'That message, abcdef, was for you; I forgot to tag you.'`"
        ]
    );
}

#[test]
fn ambiguous_advisory_never_offers_a_guessed_command() {
    let result = serde_json::json!({
        "event_id": "abcdef1234567890",
        "channel": "#mosaico",
        "coaching": [{
            "level": "warn",
            "code": "untagged_agent_prefix_ambiguous",
            "summary": "WARN: candidates are ambiguous",
            "candidates": ["drift-codex", "drizzle-codex"]
        }]
    });

    assert_eq!(
        send_coaching_lines(&result).unwrap(),
        vec!["WARN: candidates are ambiguous"]
    );
}
