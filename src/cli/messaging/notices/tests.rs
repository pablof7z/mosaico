use super::*;

#[test]
fn renderer_preserves_summaries_and_builds_an_explicit_correction_command() {
    let result = serde_json::json!({
        "event_id": "abcdef1234567890",
        "channel": "#mosaico/reviews",
        "notices": [{
            "level": "warn",
            "code": "untagged_agent_prefix",
            "summary": "WARN: published ambient chat",
            "matched_agent": "drift-codex"
        }]
    });

    assert_eq!(
        send_notice_lines(&result).unwrap(),
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
        "notices": [{
            "level": "warn",
            "code": "untagged_agent_prefix_ambiguous",
            "summary": "WARN: candidates are ambiguous",
            "candidates": ["drift-codex", "drizzle-codex"]
        }]
    });

    assert_eq!(
        send_notice_lines(&result).unwrap(),
        vec!["WARN: candidates are ambiguous"]
    );
}

#[test]
fn absent_notices_key_returns_empty_without_error() {
    let result = serde_json::json!({
        "event_id": "abc",
        "channel": "#mosaico"
    });
    assert_eq!(send_notice_lines(&result).unwrap(), Vec::<String>::new());
}
