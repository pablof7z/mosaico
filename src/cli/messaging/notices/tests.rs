use super::*;

#[test]
fn no_notices_key_returns_empty() {
    assert_eq!(
        send_coaching_lines(&serde_json::json!({})).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn renderer_preserves_summaries_and_builds_an_explicit_correction_command() {
    let result = serde_json::json!({
        "id": "abcdef",
        "notices": [{
            "level": "warn",
            "code": "untagged_agent_prefix",
            "summary": "WARN: published ambient chat",
            "matched_agent": "drift-codex",
            "channel": "#mosaico/reviews"
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
        "id": "abcdef",
        "notices": [{
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
