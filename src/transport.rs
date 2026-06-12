//! Transport — a thin adapter over `nostr-sdk` (M1 §2).
//!
//! Speaks wire events only: connect to relays (with NIP-42 auto-AUTH), publish
//! signed events, subscribe with filters, one-shot fetch for resolution. It
//! knows nothing of domain meaning — the codec owns that.
//!
//! (M1 names NMP as the eventual transport. NMP turned out to be a full
//! cross-platform app *kernel*, a poor fit for a headless CLI; the wire output
//! is identical standard Nostr either way, and this whole layer sits behind the
//! codec seam, so an NMP-backed transport remains a drop-in replacement.)

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use regex::Regex;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct Transport {
    client: Client,
    pub pubkey: PublicKey,
}

// ── secret scrubbing ──────────────────────────────────────────────────────────

static SCRUB_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

fn secret_patterns() -> &'static Vec<Regex> {
    SCRUB_PATTERNS.get_or_init(|| {
        let raw: &[&str] = &[
            // AWS access key IDs
            r"(?:AKIA|ASIA|AGPA|AIDA|AROA|AIPA|ANPA|ANVA|APKA)[0-9A-Z]{16}",
            // GitHub tokens (classic PATs, fine-grained, OAuth, runner, server)
            r"gh[pousr]_[A-Za-z0-9]{36,255}",
            // Slack tokens
            r"xox[baprs]-[A-Za-z0-9\-]{10,255}",
            // Google API keys
            r"AIza[0-9A-Za-z\-_]{35}",
            // Anthropic API keys
            r"sk-ant-[A-Za-z0-9\-_]{20,255}",
            // OpenAI / generic sk- keys (after more-specific patterns)
            r"sk-[A-Za-z0-9]{20,255}",
            // Nostr secret keys (bech32 nsec)
            r"nsec1[a-z0-9]{58}",
            // Ollama cloud API keys: <32-hex>.<20-64 alphanumeric>
            r"[a-f0-9]{32}\.[A-Za-z0-9]{20,64}",
            // PEM private key blocks
            r"-----BEGIN (?:RSA |EC |OPENSSH |PGP |DSA )?PRIVATE KEY-----",
        ];
        raw.iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    eprintln!("[tenex-edge] scrub: failed to compile pattern {p:?}: {e}");
                    None
                }
            })
            .collect()
    })
}

/// Replace detected credential spans with `[REDACTED]`. Fail-open: always
/// returns a valid string; compile errors on individual patterns are skipped.
fn scrub_secrets(input: &str) -> String {
    let mut out = input.to_string();
    for re in secret_patterns() {
        let replaced = re.replace_all(&out, "[REDACTED]");
        if let std::borrow::Cow::Owned(s) = replaced {
            out = s;
        }
    }
    out
}

/// Scrub content in-place on an `UnsignedEvent`. Resets `id` to `None` when
/// content changed so the signing step recomputes the event ID over the
/// scrubbed content (required — nostr-sdk validates id vs content on sign).
fn scrub_unsigned(unsigned: &mut UnsignedEvent) {
    if unsigned.content.is_empty() {
        return;
    }
    let scrubbed = scrub_secrets(&unsigned.content);
    if scrubbed != unsigned.content {
        eprintln!(
            "[tenex-edge] redacted secret(s) from outgoing kind:{} event",
            unsigned.kind.as_u16()
        );
        unsigned.content = scrubbed;
        unsigned.id = None; // force id recompute over scrubbed content at sign time
    }
}

// ── Transport ─────────────────────────────────────────────────────────────────

impl Transport {
    /// Connect to the configured relays and authenticate.
    pub async fn connect(relays: &[String], keys: Keys) -> Result<Self> {
        let pubkey = keys.public_key();
        let opts = ClientOptions::default().automatic_authentication(true);
        let client = Client::builder().signer(keys).opts(opts).build();
        for r in relays {
            client
                .add_relay(r)
                .await
                .with_context(|| format!("adding relay {r}"))?;
        }
        client.connect().await;
        client.wait_for_connection(Duration::from_secs(8)).await;

        // Force NIP-42 AUTH to complete BEFORE any subscription. On auth-gated
        // relays a REQ opened pre-auth is closed by the relay and never delivers
        // live events; `fetch_events` carries the auth-required retry, so this
        // warm-up authenticates the connection. No-op on open relays.
        let warmup = Filter::new().kind(Kind::from(0u16)).limit(1);
        let _ = client.fetch_events(warmup, Duration::from_secs(5)).await;

        Ok(Self { client, pubkey })
    }

    /// Sign (with the connection's key) and publish an event template.
    pub async fn publish_builder(&self, builder: EventBuilder) -> Result<EventId> {
        let out = self
            .client
            .send_event_builder(builder)
            .await
            .context("publishing event")?;
        Ok(out.val)
    }

    /// Sign with a SPECIFIC agent's keys, then publish over this (shared)
    /// connection. The per-machine daemon hosts several agent identities on one
    /// relay connection; each outgoing event must carry its true author's
    /// signature, not the connection's AUTH identity. Verified on the live relay
    /// (tests/relay_probe.rs): a B-signed event published over an A-authed
    /// connection lands under B's authorship.
    pub async fn publish_signed(&self, builder: EventBuilder, keys: &Keys) -> Result<EventId> {
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        let signed = keys.sign_event(unsigned).await.context("signing event")?;
        let out = self
            .client
            .send_event(&signed)
            .await
            .context("publishing signed event")?;
        Ok(out.val)
    }

    /// Like [`publish_signed`], but FAILS when no relay accepted the event.
    /// `send_event` resolves `Ok` even when every relay rejected (e.g. NIP-29
    /// `blocked` / `rate-limited`), reporting per-relay outcomes in `failed`.
    /// Callers that gate persistent state on a publish actually landing (NIP-29
    /// group create/membership) need that distinction, so this surfaces the
    /// relay's rejection reason as an error instead of swallowing it.
    pub async fn publish_signed_checked(&self, builder: EventBuilder, keys: &Keys) -> Result<()> {
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        let signed = keys.sign_event(unsigned).await.context("signing event")?;
        let out = self
            .client
            .send_event(&signed)
            .await
            .context("publishing signed event")?;
        if out.success.is_empty() {
            let reasons: Vec<String> = out.failed.values().cloned().collect();
            anyhow::bail!("relay rejected event: {}", reasons.join("; "));
        }
        Ok(())
    }

    /// One-shot query (used for resolution — e.g. fetch a `kind:0` profile).
    pub async fn fetch(&self, filter: Filter, timeout: Duration) -> Result<Vec<Event>> {
        let events = self
            .client
            .fetch_events(filter, timeout)
            .await
            .context("fetching events")?;
        Ok(events.into_iter().collect())
    }

    /// Open long-lived subscriptions (one per filter). Incoming events arrive on
    /// [`Transport::notifications`].
    pub async fn subscribe(&self, filters: Vec<Filter>) -> Result<()> {
        for f in filters {
            self.client
                .subscribe(f, None)
                .await
                .context("subscribing")?;
        }
        Ok(())
    }

    pub fn notifications(&self) -> broadcast::Receiver<RelayPoolNotification> {
        self.client.notifications()
    }

    pub async fn shutdown(&self) {
        self.client.disconnect().await;
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── scrub_secrets unit tests ──────────────────────────────────────────────

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
        assert!(out.contains("[REDACTED]"), "should redact ollama key; got: {out}");
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

    // ── end-to-end: scrub + sign produces a valid event ───────────────────────

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
        assert!(signed.verify().is_ok(), "signature valid over scrubbed content");
        assert!(!signed.content.contains(token), "token absent from signed event");
    }

    #[tokio::test]
    async fn scrub_unsigned_noop_on_empty_content() {
        let keys = Keys::generate();
        // Kind 9000 put-user events have empty content — must be untouched.
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
