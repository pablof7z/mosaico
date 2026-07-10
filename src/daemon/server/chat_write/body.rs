use nostr_sdk::prelude::{PublicKey, ToBech32};

pub(super) fn rewrite_first_resolved_mention(body: &str, raw_label: &str, pubkey: &str) -> String {
    let Some((start, end)) = first_mention_span(body, raw_label) else {
        return body.to_string();
    };
    let Ok(pk) = PublicKey::parse(pubkey) else {
        return body.to_string();
    };
    let npub = pk.to_bech32().expect("public key encodes as npub");
    let mut out = String::with_capacity(body.len() + npub.len());
    out.push_str(&body[..start]);
    out.push_str("nostr:");
    out.push_str(&npub);
    out.push_str(&body[end..]);
    out
}

fn first_mention_span(body: &str, raw_label: &str) -> Option<(usize, usize)> {
    for (at, _) in body.match_indices('@') {
        if at > 0
            && body[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        let start = at + 1;
        let after = &body[start..];
        let end_rel = after
            .find(|c: char| {
                !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '@'))
            })
            .unwrap_or(after.len());
        let mut end = start + end_rel;
        while end > start && matches!(body[..end].chars().next_back(), Some('.' | '@' | '/')) {
            end -= body[..end]
                .chars()
                .next_back()
                .map(char::len_utf8)
                .unwrap_or(1);
        }
        if &body[start..end] == raw_label {
            return Some((at, end));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET_PK: &str = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";

    #[test]
    fn rewrites_resolved_handle_to_nostr_entity() {
        let out = rewrite_first_resolved_mention(
            "@flint-range-108@laptop heads up",
            "flint-range-108@laptop",
            TARGET_PK,
        );

        assert!(out.starts_with("nostr:npub1"));
        assert!(out.ends_with(" heads up"));
        assert!(!out.contains("@flint-range-108@laptop"));
    }

    #[test]
    fn preserves_punctuation_after_mention() {
        let out = rewrite_first_resolved_mention("ping @codex.", "codex", TARGET_PK);

        assert!(out.starts_with("ping nostr:npub1"));
        assert!(out.ends_with('.'));
    }

    #[test]
    fn rewrites_full_agent_session_handle() {
        let out =
            rewrite_first_resolved_mention("hey @codex/echo123 now", "codex/echo123", TARGET_PK);

        assert!(out.starts_with("hey nostr:npub1"));
        assert!(out.ends_with(" now"));
        assert!(!out.contains("@codex"));
        assert!(!out.contains("/echo123"));
    }

    #[test]
    fn skips_email_like_substrings() {
        let out =
            rewrite_first_resolved_mention("mail dev@codex first, then @codex", "codex", TARGET_PK);

        assert!(out.starts_with("mail dev@codex first, then nostr:npub1"));
    }
}
