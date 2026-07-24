//! Product credential scrubbing applied before NMP freezes a write.

use nostr::*;

/// Scrub content in-place on an `UnsignedEvent`. Resets `id` to `None` when
/// content changed so the signing step recomputes the event ID over the
/// scrubbed content (required — nostr validates id vs content on sign).
pub(crate) fn scrub_unsigned(unsigned: &mut UnsignedEvent) {
    if unsigned.content.is_empty() {
        return;
    }
    let scrubbed = crate::secret_scrub::scrub(&unsigned.content);
    if scrubbed != unsigned.content {
        eprintln!(
            "[mosaico] redacted secret(s) from outgoing kind:{} event",
            unsigned.kind.as_u16()
        );
        unsigned.content = scrubbed;
        unsigned.id = None; // force id recompute over scrubbed content at sign time
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_aws_key() {
        let input = "my key is AKIAIOSFODNN7EXAMPLE right there";
        let out = crate::secret_scrub::scrub(input);
        assert!(out.contains("[REDACTED]"), "should redact AWS key");
        assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "should not leak key");
    }

    #[test]
    fn redacts_github_pat() {
        let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
        let input = format!("use this token: {token}");
        let out = crate::secret_scrub::scrub(&input);
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains(token));
    }

    #[test]
    fn redacts_anthropic_key() {
        let key = "sk-ant-api03-abc123def456ghi789jkl012mno345pqr";
        let out = crate::secret_scrub::scrub(key);
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("sk-ant-"));
    }

    #[test]
    fn redacts_openai_key() {
        let key = "sk-abcdefghijklmnopqrstuvwxyz123456";
        let out = crate::secret_scrub::scrub(key);
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains(key));
    }

    #[test]
    fn redacts_nsec() {
        // bech32-encoded nostr secret key: "nsec1" + 58 lowercase alphanumeric chars
        let nsec = format!("nsec1{}", "a".repeat(58));
        let input = format!("my key: {nsec}");
        let out = crate::secret_scrub::scrub(&input);
        assert!(out.contains("[REDACTED]"), "should redact nsec; got: {out}");
        assert!(!out.contains("nsec1"), "should not leak nsec prefix");
    }

    #[test]
    fn redacts_ollama_key() {
        let key = "e1a1e08cbfbc4bbf9b702162cbdbd0f6.qQmkiYini5T9hrtMgiYalWb2";
        let input = format!("my ollama key is {key}");
        let out = crate::secret_scrub::scrub(&input);
        assert!(
            out.contains("[REDACTED]"),
            "should redact ollama key; got: {out}"
        );
        assert!(!out.contains(key));
    }

    #[test]
    fn does_not_redact_plain_text() {
        let input = "please refactor the akimbo module and fix the bug";
        let out = crate::secret_scrub::scrub(input);
        assert_eq!(out, input, "ordinary text must pass through unchanged");
    }

    #[test]
    fn empty_content_unchanged() {
        assert_eq!(crate::secret_scrub::scrub(""), "");
    }

    #[tokio::test]
    async fn scrub_unsigned_then_sign_verifies() {
        let keys = Keys::generate();
        let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
        let raw_content = format!("leak {token} in prompt");

        let builder = EventBuilder::new(Kind::from(1u16), raw_content);
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);

        assert!(unsigned.content.contains("[REDACTED]"), "content scrubbed");
        assert!(!unsigned.content.contains(token), "token absent");

        // Sign and verify: nostr recomputes the id from scrubbed content
        // on sign, so verify() must succeed (proves id reset was applied).
        let signed = keys.sign_event(unsigned).await.expect("signing");
        assert!(
            signed.verify().is_ok(),
            "signature valid over scrubbed content"
        );
        assert!(
            !signed.content.contains(token),
            "token absent from signed event"
        );
    }

    #[tokio::test]
    async fn scrub_unsigned_noop_on_empty_content() {
        let keys = Keys::generate();
        // Kind 9000 put-user events have empty content; it must be untouched.
        let builder = EventBuilder::new(Kind::from(9000u16), "");
        let mut unsigned = builder.build(keys.public_key());
        let original_id = unsigned.id;
        scrub_unsigned(&mut unsigned);
        assert_eq!(
            unsigned.id, original_id,
            "id must not be reset when content is empty"
        );
    }
}
