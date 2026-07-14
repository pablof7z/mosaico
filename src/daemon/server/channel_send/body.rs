use super::TaggedRecipient;
use anyhow::{Context, Result};
use nostr_sdk::prelude::{PublicKey, ToBech32};

pub(super) fn format_tagged_body(message: &str, tagged: &[TaggedRecipient]) -> Result<String> {
    if tagged.is_empty() {
        return Ok(message.to_string());
    }
    let addresses = tagged
        .iter()
        .map(|target| {
            let public_key = PublicKey::parse(&target.pubkey)
                .with_context(|| format!("invalid pubkey for --tag {:?}", target.label))?;
            Ok(format!("nostr:{}", public_key.to_bech32()?))
        })
        .collect::<Result<Vec<_>>>()?;
    let message = strip_existing_tag_prefix(message, tagged);
    Ok(format!("{}: {message}", addresses.join(", ")))
}

fn strip_existing_tag_prefix<'a>(message: &'a str, tagged: &[TaggedRecipient]) -> &'a str {
    let Some((prefix, body)) = message.split_once(':') else {
        return message;
    };
    let already_addressed = prefix.split(',').all(|part| {
        let Some(label) = part.trim().strip_prefix('@') else {
            return false;
        };
        !label.is_empty() && tagged.iter().any(|target| target.label == label)
    });
    if already_addressed {
        body.strip_prefix(' ').unwrap_or(body)
    } else {
        message
    }
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
            run_id: None,
            channel: "root".to_string(),
        }
    }

    #[test]
    fn adds_one_nostr_address_prefix() {
        let body = format_tagged_body("hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.starts_with("nostr:npub1"));
        assert!(body.ends_with(": hello"));
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

        assert_eq!(body.matches("nostr:npub1").count(), 2);
        assert!(body.contains(", nostr:npub1"));
        assert!(body.ends_with(": hello"));
    }

    #[test]
    fn replaces_an_existing_agent_prefix_instead_of_duplicating_it() {
        let body = format_tagged_body("@agent1: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert_eq!(
            body.matches(':').count(),
            2,
            "one in nostr and one separator"
        );
        assert!(body.ends_with(": hello"));
        assert!(!body.contains("@agent1"));
    }

    #[test]
    fn preserves_unrelated_leading_address_text() {
        let body = format_tagged_body("@human: hello", &[recipient("agent1", FIRST_PK)]).unwrap();

        assert!(body.ends_with(": @human: hello"));
    }

    #[test]
    fn tagged_body_preserves_other_inline_handles_literally() {
        let body = format_tagged_body(
            "hello, @a2 keeps ignoring me today",
            &[recipient("a1", FIRST_PK)],
        )
        .unwrap();

        assert!(body.starts_with("nostr:npub1"));
        assert!(body.ends_with(": hello, @a2 keeps ignoring me today"));
    }
}
