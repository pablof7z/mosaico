use super::TaggedRecipient;
use anyhow::{Context, Result};
use nostr::{PublicKey, ToBech32};

pub(super) struct FormattedBody {
    pub(super) wire: String,
    pub(super) message: String,
    pub(super) stripped_label: Option<String>,
}

pub(super) fn format_tagged_body(
    message: &str,
    tagged: &[TaggedRecipient],
) -> Result<FormattedBody> {
    if tagged.is_empty() {
        return Ok(FormattedBody {
            wire: message.to_string(),
            message: message.to_string(),
            stripped_label: None,
        });
    }
    let addresses = tagged
        .iter()
        .map(|target| {
            let public_key = PublicKey::parse(&target.pubkey)
                .with_context(|| format!("invalid pubkey for --tag {:?}", target.label))?;
            Ok(format!("nostr:{}", public_key.to_bech32()?))
        })
        .collect::<Result<Vec<_>>>()?;
    let (message, stripped_label) = strip_existing_tag_prefix(message, tagged);
    Ok(FormattedBody {
        wire: format!("{}: {message}", addresses.join(", ")),
        message: message.to_string(),
        stripped_label,
    })
}

fn strip_existing_tag_prefix<'a>(
    message: &'a str,
    tagged: &[TaggedRecipient],
) -> (&'a str, Option<String>) {
    let trimmed = message.trim_start();
    if let Some(rest) = trimmed.strip_prefix('@') {
        let label_end = rest
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ':' | ','))
            .unwrap_or(rest.len());
        let (label, suffix) = rest.split_at(label_end);
        let Some(tagged_label) = matching_label(label, tagged) else {
            return (message, None);
        };
        let remainder = match suffix.chars().next() {
            Some(':') | Some(',') => suffix[1..].trim_start(),
            Some(ch) if ch.is_whitespace() => suffix.trim_start(),
            None => "",
            _ => return (message, None),
        };
        return (remainder, Some(tagged_label.to_string()));
    }

    let Some((label, suffix)) = trimmed.split_once(':') else {
        return (message, None);
    };
    if label.is_empty() || label.chars().any(char::is_whitespace) {
        return (message, None);
    }
    let Some(tagged_label) = matching_label(label, tagged) else {
        return (message, None);
    };
    (suffix.trim_start(), Some(tagged_label.to_string()))
}

fn matching_label<'a>(label: &str, tagged: &'a [TaggedRecipient]) -> Option<&'a str> {
    (!label.is_empty())
        .then(|| {
            tagged
                .iter()
                .find(|target| target.label.eq_ignore_ascii_case(label))
                .map(|target| target.label.as_str())
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST_PK: &str = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";
    const SECOND_PK: &str = "83d3c36a3b1f1d96a65a506c965d185a02d3145039e0c0056014e366474f83aa";

    fn recipient(label: &str, pubkey: &str) -> TaggedRecipient {
        TaggedRecipient {
            label: label.to_string(),
            pubkey: pubkey.to_string(),
            channel: "root".to_string(),
        }
    }

    #[test]
    fn adds_one_nostr_address_prefix() {
        let body = format_tagged_body("hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.wire.starts_with("nostr:npub1"));
        assert!(body.wire.ends_with(": hello"));
        assert_eq!(body.message, "hello");
        assert_eq!(body.stripped_label, None);
    }

    #[test]
    fn adds_multiple_nostr_address_prefixes_in_tag_order() {
        let body = format_tagged_body(
            "hello",
            &[
                recipient("agent1", FIRST_PK),
                recipient("agent2", SECOND_PK),
            ],
        )
        .unwrap();

        assert_eq!(body.wire.matches("nostr:npub1").count(), 2);
        assert!(body.wire.contains(", nostr:npub1"));
        assert!(body.wire.ends_with(": hello"));
    }

    #[test]
    fn replaces_an_existing_agent_prefix_instead_of_duplicating_it() {
        let body = format_tagged_body("@agent1: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert_eq!(
            body.wire.matches(':').count(),
            2,
            "one in nostr and one separator"
        );
        assert!(body.wire.ends_with(": hello"));
        assert!(!body.wire.contains("@agent1"));
        assert_eq!(body.stripped_label.as_deref(), Some("agent1"));
    }

    #[test]
    fn strips_supported_leading_tag_forms_case_insensitively() {
        for message in ["@agent1 hello", "@agent1, hello", "@Agent1: hello"] {
            let body = format_tagged_body(message, &[recipient("agent1", FIRST_PK)]).unwrap();
            assert!(body.wire.ends_with(": hello"), "got {}", body.wire);
            assert!(
                !body.wire.to_ascii_lowercase().contains("@agent1"),
                "got {}",
                body.wire
            );
        }
    }

    #[test]
    fn strips_a_bare_at_matching_tag() {
        let body = format_tagged_body("@agent1", &[recipient("agent1", FIRST_PK)]).unwrap();
        assert!(body.wire.ends_with(": "), "got {}", body.wire);
        assert!(!body.wire.contains("@agent1"), "got {}", body.wire);
    }

    #[test]
    fn strips_a_bare_label_and_colon_case_insensitively() {
        let body = format_tagged_body("Agent1: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.wire.ends_with(": hello"), "got {}", body.wire);
        assert_eq!(body.message, "hello");
        assert_eq!(body.stripped_label.as_deref(), Some("agent1"));
    }

    #[test]
    fn strips_at_most_one_leading_matching_tag() {
        let body = format_tagged_body("@agent1: @agent1: hello", &[recipient("agent1", FIRST_PK)])
            .unwrap();

        assert!(body.wire.ends_with(": @agent1: hello"), "got {}", body.wire);
    }

    #[test]
    fn preserves_unrelated_leading_address_text() {
        let body = format_tagged_body("@human: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.wire.ends_with(": @human: hello"));
        assert_eq!(body.stripped_label, None);
    }

    #[test]
    fn preserves_unrelated_bare_label_text() {
        let body = format_tagged_body("human: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.wire.ends_with(": human: hello"));
        assert_eq!(body.stripped_label, None);
    }

    #[test]
    fn tagged_body_preserves_other_inline_handles_literally() {
        let body = format_tagged_body(
            "hello, @a2 keeps ignoring me today",
            &[recipient("a1", FIRST_PK)],
        )
        .unwrap();

        assert!(body.wire.starts_with("nostr:npub1"));
        assert!(body.wire.ends_with(": hello, @a2 keeps ignoring me today"));
    }
}
