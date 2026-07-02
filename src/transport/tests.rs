use super::*;

// ── stable subscription id ──────────────────────────────────────────────────

#[test]
fn stable_subscription_id_is_deterministic_and_distinct() {
    let pk = Keys::generate().public_key();
    let a1 = Filter::new().kind(Kind::from(0u16)).author(pk);
    let a2 = Filter::new().kind(Kind::from(0u16)).author(pk);
    let b = Filter::new().kind(Kind::from(9u16)).author(pk);

    // Same filter content -> identical id, so resubscribe replaces rather than
    // accumulating a fresh random-id subscription.
    assert_eq!(stable_subscription_id(&a1), stable_subscription_id(&a2));
    assert_ne!(stable_subscription_id(&a1), stable_subscription_id(&b));
}

// ── assert_relay_accepted unit tests ────────────────────────────────────────

fn output_with(success: &[&str], failed: &[(&str, &str)]) -> Output<EventId> {
    let mut out: Output<EventId> = Output {
        val: EventId::all_zeros(),
        success: Default::default(),
        failed: Default::default(),
    };
    for url in success {
        out.success.insert(RelayUrl::parse(url).unwrap());
    }
    for (url, reason) in failed {
        out.failed
            .insert(RelayUrl::parse(url).unwrap(), reason.to_string());
    }
    out
}

#[test]
fn accepted_when_any_relay_succeeds() {
    let out = output_with(&["wss://ok.relay"], &[("wss://bad.relay", "blocked")]);
    assert!(assert_relay_accepted(&out, None).is_ok());
}

#[test]
fn duplicate_response_counts_as_already_accepted() {
    let out = output_with(&[], &[("wss://ok.relay", "duplicate: already have event")]);
    assert!(assert_relay_accepted(&out, None).is_ok());
}

#[test]
fn rejected_surfaces_relay_reason() {
    let out = output_with(&[], &[("wss://nip29.relay", "blocked: unknown member")]);
    let err = assert_relay_accepted(&out, None).unwrap_err().to_string();
    assert!(err.contains("blocked: unknown member"), "got: {err}");
}

#[test]
fn no_accept_no_reason_reports_timeout() {
    // Every relay silent: send_event resolved Ok but no OK,true ever arrived.
    let out = output_with(&[], &[]);
    let err = assert_relay_accepted(&out, None).unwrap_err().to_string();
    assert!(err.contains("no relay accepted"), "got: {err}");
}

// ── scrub_secrets unit tests ────────────────────────────────────────────────

/// Fail-loud guard: every credential pattern MUST compile. At runtime a broken
/// pattern is silently skipped (fail-open), so this build-time assertion is the
/// only thing standing between a typo and a disabled credential class.
#[test]
fn all_scrub_patterns_compile() {
    for p in SECRET_PATTERN_SOURCES {
        assert!(
            regex::Regex::new(p).is_ok(),
            "scrub pattern failed to compile: {p:?}"
        );
    }
    // And the live set actually built one regex per source (no silent drops).
    assert_eq!(secret_patterns().len(), SECRET_PATTERN_SOURCES.len());
}

#[test]
fn redacts_aws_key() {
    let input = "my key is AKIAIOSFODNN7EXAMPLE right there";
    let out = scrub_secrets(input);
    assert!(out.contains("[REDACTED]"), "should redact AWS key");
    assert!(!out.contains("AKIAIOSFODNN7EXAMPLE"), "should not leak key");
}

#[test]
fn redacts_github_pat() {
    let token = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
    let input = format!("use this token: {token}");
    let out = scrub_secrets(&input);
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains(token));
}

#[test]
fn redacts_anthropic_key() {
    let key = "sk-ant-api03-abc123def456ghi789jkl012mno345pqr";
    let out = scrub_secrets(key);
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("sk-ant-"));
}

#[test]
fn redacts_openai_key() {
    let key = "sk-abcdefghijklmnopqrstuvwxyz123456";
    let out = scrub_secrets(key);
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains(key));
}

#[test]
fn redacts_nsec() {
    // bech32-encoded nostr secret key: "nsec1" + 58 lowercase alphanumeric chars
    let nsec = format!("nsec1{}", "a".repeat(58));
    let input = format!("my key: {nsec}");
    let out = scrub_secrets(&input);
    assert!(out.contains("[REDACTED]"), "should redact nsec; got: {out}");
    assert!(!out.contains("nsec1"), "should not leak nsec prefix");
}

#[test]
fn redacts_ollama_key() {
    let key = "e1a1e08cbfbc4bbf9b702162cbdbd0f6.qQmkiYini5T9hrtMgiYalWb2";
    let input = format!("my ollama key is {key}");
    let out = scrub_secrets(&input);
    assert!(
        out.contains("[REDACTED]"),
        "should redact ollama key; got: {out}"
    );
    assert!(!out.contains(key));
}

#[test]
fn does_not_redact_plain_text() {
    let input = "please refactor the akimbo module and fix the bug";
    let out = scrub_secrets(input);
    assert_eq!(out, input, "ordinary text must pass through unchanged");
}

#[test]
fn empty_content_unchanged() {
    assert_eq!(scrub_secrets(""), "");
}

// ── end-to-end: scrub + sign produces a valid event ─────────────────────────

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

    // Sign and verify: nostr-sdk recomputes the id from scrubbed content
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
