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
    /// URL of the profile indexer relay (READ-only), if configured. kind:0
    /// profiles are routed here via [`Transport::publish_signed_to`]; all other
    /// events go to the WRITE relays (main NIP-29 relay) via `send_event`, which
    /// skips READ-only relays. This prevents the indexer from rejecting NIP-29
    /// kinds ("blocked: kind 9000 is not allowed") and having that rejection
    /// pollute `assert_relay_accepted`'s joined-reason verdict.
    indexer_url: Option<String>,
}

// ── secret scrubbing ──────────────────────────────────────────────────────────

static SCRUB_PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();

/// Credential-detection regexes, ordered most-specific first. A pattern that
/// fails to compile silently disables scrubbing for that credential class (a
/// fail-open leak risk), so the `all_scrub_patterns_compile` test asserts every
/// source here compiles — a broken pattern fails the build loudly rather than
/// at runtime.
pub(crate) const SECRET_PATTERN_SOURCES: &[&str] = &[
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

fn secret_patterns() -> &'static Vec<Regex> {
    SCRUB_PATTERNS.get_or_init(|| {
        SECRET_PATTERN_SOURCES
            .iter()
            .filter_map(|p| match Regex::new(p) {
                Ok(re) => Some(re),
                Err(e) => {
                    // Runtime stays safe (we skip the broken pattern), but the
                    // `all_scrub_patterns_compile` test guarantees we never ship one.
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
pub(crate) fn scrub_unsigned(unsigned: &mut UnsignedEvent) {
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

// ── relay-ack assertion ─────────────────────────────────────────────────────

/// Fail unless at least one relay accepted the publish. `nostr-sdk`'s
/// `send_event*` resolves `Ok` as long as the message was transmitted; the
/// actual NIP-01 `["OK", id, true|false, reason]` verdict per relay lives in
/// `output.success` / `output.failed`. An empty `success` set usually means every
/// relay rejected the event (or the connection timed out before any OK arrived),
/// so a caller reporting "published" off the bare `Ok` would be lying. A
/// "duplicate" failure is the idempotent exception: the relay already has the
/// signed event, so durability is satisfied.
fn assert_relay_accepted(output: &Output<EventId>, event: Option<&Event>) -> Result<()> {
    if !output.success.is_empty() {
        return Ok(());
    }
    if output
        .failed
        .values()
        .any(|r| r.to_ascii_lowercase().contains("duplicate"))
    {
        return Ok(());
    }
    let reasons: Vec<String> = output
        .failed
        .values()
        .filter(|r| !r.is_empty())
        .cloned()
        .collect();
    if reasons.is_empty() {
        crate::relay_log::log_relay_rejection("no relay returned OK (timeout)", event);
        anyhow::bail!("no relay accepted the event (timeout or no OK received)");
    }
    let msg = reasons.join("; ");
    crate::relay_log::log_relay_rejection(&msg, event);
    anyhow::bail!("relay rejected event: {msg}");
}

// ── Transport ─────────────────────────────────────────────────────────────────

impl Transport {
    /// Connect to the configured relays and authenticate.
    pub async fn connect(relays: &[String], keys: Keys) -> Result<Self> {
        Self::connect_with_indexer(relays, None, keys).await
    }

    /// Connect to the configured main relays (READ+WRITE) plus an optional
    /// indexer relay (READ-only). The indexer receives kind:0 profile publishes
    /// via [`Transport::publish_signed_to`] and serves kind:0 lookups, but is
    /// excluded from `send_event` broadcasts (which target WRITE relays only),
    /// so it never sees — and therefore never rejects — NIP-29 group events.
    pub async fn connect_with_indexer(
        relays: &[String],
        indexer_url: Option<&str>,
        keys: Keys,
    ) -> Result<Self> {
        let pubkey = keys.public_key();
        let opts = ClientOptions::default().automatic_authentication(true);
        let client = Client::builder().signer(keys).opts(opts).build();
        for r in relays {
            client
                .add_relay(r)
                .await
                .with_context(|| format!("adding relay {r}"))?;
        }
        // Add the indexer as READ-only so send_event (which targets WRITE relays)
        // skips it. kind:0 profiles route to it explicitly via publish_signed_to.
        if let Some(url) = indexer_url {
            if !url.is_empty() {
                client
                    .add_read_relay(url)
                    .await
                    .with_context(|| format!("adding indexer relay {url} (READ-only)"))?;
            }
        }
        // Kick off the connection in the BACKGROUND (non-blocking) and return
        // immediately. Awaiting connectivity + NIP-42 auth is `warmup()`'s job,
        // which the daemon runs OFF its startup critical path so store-only RPCs
        // (`who`, `tmux`, chat/inbox reads) serve instantly even when the relay
        // is slow or unreachable.
        client.connect().await;
        Ok(Self {
            client,
            pubkey,
            indexer_url: indexer_url.filter(|s| !s.is_empty()).map(String::from),
        })
    }

    /// Block (bounded) until the relay connection is established, then force
    /// NIP-42 AUTH to complete BEFORE any subscription is opened. On auth-gated
    /// relays a REQ opened pre-auth is closed by the relay and never delivers
    /// live events; the warm-up `fetch_events` carries the auth-required retry.
    /// No-op on open relays. Intended to run in a background task at startup.
    pub async fn warmup(&self) {
        self.client
            .wait_for_connection(Duration::from_secs(8))
            .await;
        let warmup = Filter::new().kind(Kind::from(0u16)).limit(1);
        let _ = self
            .client
            .fetch_events(warmup, Duration::from_secs(5))
            .await;
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

    /// Like [`publish_builder`], but FAILS when no relay accepted the event.
    /// `send_event_builder` resolves `Ok` even when every relay rejected (the
    /// per-relay verdict lives in `success`/`failed`), so the bare
    /// [`publish_builder`] reports an optimistic write-side ack rather than a
    /// confirmed NIP-01 `OK,true`. Use this whenever a green result must mean the
    /// relay actually stored the event.
    pub async fn publish_builder_checked(&self, builder: EventBuilder) -> Result<EventId> {
        let out = self
            .client
            .send_event_builder(builder)
            .await
            .context("publishing event")?;
        assert_relay_accepted(&out, None)?;
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
        crate::relay_log::log_outgoing_event(&signed);
        let out = self
            .client
            .send_event(&signed)
            .await
            .context("publishing signed event")?;
        Ok(out.val)
    }

    /// Like [`publish_signed`], but FAILS when no relay accepted the event and
    /// returns the published [`EventId`] on success. `send_event` resolves `Ok`
    /// even when every relay rejected (e.g. NIP-29 `blocked` / `rate-limited`),
    /// reporting per-relay outcomes in `failed`. Callers that gate persistent
    /// state on a publish actually landing (NIP-29 group create/membership,
    /// long-form proposals) need that distinction, so this surfaces the relay's
    /// rejection reason as an error instead of swallowing it.
    ///
    /// No hidden retry is queued here. A checked publish is a verdict; the caller
    /// owns any domain-specific retry policy.
    pub async fn publish_signed_checked(
        &self,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        let signed = keys.sign_event(unsigned).await.context("signing event")?;
        crate::relay_log::log_outgoing_event(&signed);
        let out = self
            .client
            .send_event(&signed)
            .await
            .context("publishing signed event")?;
        assert_relay_accepted(&out, Some(&signed))?;
        Ok(out.val)
    }

    /// Sign `builder` with `keys` and return the signed event WITHOUT publishing.
    /// Lets a caller learn the final `EventId` and act on it (e.g. record it as
    /// already-seen) BEFORE the wire send, so the relay echo cannot race ahead.
    /// Wire-identical to [`publish_signed`] up to the `send_event` call.
    pub async fn sign(&self, builder: EventBuilder, keys: &Keys) -> Result<Event> {
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        keys.sign_event(unsigned).await.context("signing event")
    }

    /// Publish an already-signed event (see [`sign`]).
    pub async fn publish_event(&self, signed: &Event) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(signed);
        let out = self
            .client
            .send_event(signed)
            .await
            .context("publishing signed event")?;
        Ok(out.val)
    }

    /// Like [`publish_event`], but FAILS when no relay accepted the event.
    /// `send_event` resolves `Ok` even when every relay rejected (e.g. NIP-29
    /// `blocked` / `rate-limited`); a caller that reports success on a bare `Ok`
    /// would mask a silently-dropped event. Use this whenever a reported event id
    /// must mean the event is actually on the relay — the canonical case is
    /// `channels_create`, which returns `orchestration_event_id` to the operator
    /// and drives a local fast-path handler off the same event. The caller owns
    /// any retry policy; this function reports the relay verdict once.
    pub async fn publish_event_checked(&self, signed: &Event) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(signed);
        let out = self
            .client
            .send_event(signed)
            .await
            .context("publishing signed event")?;
        assert_relay_accepted(&out, Some(signed))?;
        Ok(out.val)
    }

    /// Publish an already-signed event to a specific relay subset (by URL).
    /// Used by the indexer publish path: kind:0 profiles go to the READ-only
    /// indexer relay, which is NOT in the WRITE set targeted by `send_event`.
    /// Falls back to broadcasting on all WRITE relays when no indexer is
    /// configured (preserves behavior for single-relay dev setups).
    pub async fn publish_event_to(&self, signed: &Event, urls: &[String]) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(signed);
        if urls.is_empty() {
            // No explicit targets — broadcast on WRITE relays (the default pool).
            let out = self
                .client
                .send_event(signed)
                .await
                .context("publishing signed event")?;
            return Ok(out.val);
        }
        let out = self
            .client
            .send_event_to(urls.iter().cloned(), signed)
            .await
            .context("publishing signed event to target relays")?;
        assert_relay_accepted(&out, Some(signed))?;
        Ok(out.val)
    }

    /// The configured indexer relay URL, if any. kind:0 profiles route here.
    pub fn indexer_url(&self) -> Option<&str> {
        self.indexer_url.as_deref()
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
    ///
    /// Each filter is subscribed under a DETERMINISTIC [`SubscriptionId`] derived
    /// from its content, so re-subscribing the same filter REPLACES the existing
    /// relay subscription (the pool keys `SubscriptionData` by id) instead of
    /// opening a new one. `Client::subscribe(f, None)` mints a fresh random id on
    /// every call; the daemon's `resubscribe` runs on every session_start/spawn,
    /// so random ids leaked an unbounded number of subscriptions into the relay
    /// pool — each retaining a full clone of the filter (BTreeMaps of tag/pubkey/
    /// kind sets) — growing the process to tens of GB over a day. Stable ids cap
    /// the live set at the actual working set (projects × agents × kinds).
    pub async fn subscribe(&self, filters: Vec<Filter>) -> Result<()> {
        for f in filters {
            let id = stable_subscription_id(&f);
            self.client
                .subscribe_with_id(id, f, None)
                .await
                .context("subscribing")?;
        }
        Ok(())
    }

    /// Subscribe a single filter under an EXPLICIT (caller-chosen, e.g. semantic)
    /// [`SubscriptionId`]. Re-subscribing the same id REPLACES the existing relay
    /// subscription in place (NIP-01: a REQ with a known id edits it), so the
    /// caller can intentionally retire/replace a role.
    ///
    /// This differs from [`Transport::subscribe`] in WHERE the id comes from.
    /// `subscribe` derives the id from filter CONTENT (a leak guard — identical
    /// filters collapse to one subscription). Here the id is owned by the caller
    /// (the subscription registry), which wants the opposite control: a *stable
    /// name for a role* whose filter changes over time. Keying by content would
    /// strand the old filter under a now-orphaned id every time the role's filter
    /// shifted; keying by the registry's id lets a single CLOSE/replace swap it.
    pub async fn subscribe_with_id(&self, id: SubscriptionId, filter: Filter) -> Result<()> {
        self.client
            .subscribe_with_id(id, filter, None)
            .await
            .context("subscribing")?;
        Ok(())
    }

    /// Like [`Transport::subscribe_with_id`], but restricts the subscription to
    /// `relays` (a subset of the connected pool). The registry uses this to keep
    /// broad `#h`/`#p` live subscriptions on the main relays and OFF the profile
    /// indexer relay — that relay is a one-shot `kind:0` resolution endpoint, and
    /// pinning firehose filters there wastes its connection and pulls in noise.
    ///
    /// If `relays` is empty, this falls back to subscribing on ALL connected
    /// relays (identical to `subscribe_with_id`) rather than silently
    /// subscribing to nothing — an empty target set is a caller convenience
    /// ("everywhere"), not an instruction to drop the subscription.
    pub async fn subscribe_with_id_to(
        &self,
        relays: &[String],
        id: SubscriptionId,
        filter: Filter,
    ) -> Result<()> {
        if relays.is_empty() {
            self.client
                .subscribe_with_id(id, filter, None)
                .await
                .context("subscribing")?;
        } else {
            // Materialize an OWNED relay list before the await. Passing a borrowed
            // iterator with a `|s| s.as_str()` closure (`&String -> &str`) into the
            // async sdk call keeps a higher-ranked borrow alive across the await
            // point; when this future is `tokio::spawn`ed (as it is from
            // `resubscribe`/`ensure_subscription`), that trips the compiler's
            // "implementation of Send/FnOnce is not general enough" limitation.
            // An owned `Vec<String>` (String: TryIntoUrl) carries no borrow.
            let urls: Vec<String> = relays.to_vec();
            self.client
                .subscribe_with_id_to(urls, id, filter, None)
                .await
                .context("subscribing to relays")?;
        }
        Ok(())
    }

    /// Close a subscription by id (sends a NIP-01 CLOSE to the relays). The
    /// codebase had no unsubscribe path before — every subscription lived until
    /// the client disconnected. The registry needs this to compact narrow REQs
    /// and retire stale subscriptions as roles come and go, instead of letting
    /// the relay-side subscription set grow monotonically. The sdk's
    /// `unsubscribe` returns unit (best-effort fire-and-forget); we wrap it as
    /// `Ok` so callers share one fallible signature across the registry surface.
    pub async fn unsubscribe(&self, id: &SubscriptionId) -> Result<()> {
        self.client.unsubscribe(id).await;
        Ok(())
    }

    pub fn notifications(&self) -> broadcast::Receiver<RelayPoolNotification> {
        self.client.notifications()
    }

    pub async fn shutdown(&self) {
        self.client.disconnect().await;
    }
}

/// Deterministic [`SubscriptionId`] for a filter: same filter → same id, so a
/// repeated `subscribe` updates the existing relay subscription in place rather
/// than minting a new random-id one. `DefaultHasher` is fixed-seed (no per-run
/// randomization), so the id is stable across daemon restarts. A 64-bit hash is
/// ample for the few hundred distinct filters a daemon ever holds.
fn stable_subscription_id(filter: &Filter) -> SubscriptionId {
    use std::hash::{Hash, Hasher};
    let json = serde_json::to_string(filter).unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    json.hash(&mut hasher);
    SubscriptionId::new(format!("te-{:016x}", hasher.finish()))
}

#[cfg(test)]
mod tests;
