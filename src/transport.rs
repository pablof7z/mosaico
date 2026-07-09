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
use std::time::Duration;
use tokio::sync::broadcast;

mod scrub;
use scrub::scrub_unsigned;

pub struct Transport {
    client: Client,
    pub pubkey: PublicKey,
    /// URLs of the main NIP-29 relay(s) — the explicit broadcast target for
    /// every publish. `nostr-relay-pool` gates BOTH `send_event`'s implicit
    /// "all WRITE-flagged relays" broadcast AND an explicitly-targeted
    /// `send_event_to` by the SAME per-relay `WRITE` flag (there is no flag
    /// combination meaning "writable only when explicitly addressed, excluded
    /// from broadcast"). Since the indexer relay must carry `WRITE` (so the
    /// explicit kind:0 publish below can reach it at all), publishing here
    /// MUST target `write_relay_urls` explicitly rather than call the
    /// pool's implicit broadcast — otherwise every publish would fan out to
    /// the indexer too and it would reject non-kind:0 events.
    write_relay_urls: Vec<String>,
    /// URL of the profile indexer relay, if configured. kind:0 profiles are
    /// routed here explicitly via [`Transport::publish_event_to`]; it is added
    /// with full READ+WRITE flags (needed for that explicit publish to
    /// succeed) but is deliberately excluded from `write_relay_urls`, so it
    /// never receives the broadcast publishes above and therefore never
    /// rejects NIP-29 kinds ("blocked: kind 9000 is not allowed") — which
    /// would otherwise pollute `assert_relay_accepted`'s joined-reason verdict.
    indexer_url: Option<String>,
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

/// `connect_with_indexer` kicks off the relay connection in the background and
/// returns immediately (see its doc), and the daemon opens its RPC accept loop
/// before that connection finishes (relay warmup runs off the startup critical
/// path too) — so a client that races a freshly-spawned daemon can reach a
/// publish call before any relay has finished its handshake. `nostr-relay-pool`
/// treats that as a hard per-relay failure (`Error::NotConnected`) rather than
/// queuing, so every publish path below waits (briefly, bounded) for the
/// connection first. Once connected this returns immediately, so it's free on
/// the steady-state path.
const PUBLISH_CONNECT_WAIT: Duration = Duration::from_secs(8);

impl Transport {
    /// Connect to the configured relays and authenticate.
    pub async fn connect(relays: &[String], keys: Keys) -> Result<Self> {
        Self::connect_with_indexer(relays, None, keys).await
    }

    /// Connect to the configured main relays plus an optional profile indexer
    /// relay. The indexer is added with full READ+WRITE flags (both are needed:
    /// READ for kind:0 lookups, WRITE for the explicit kind:0 publish), but
    /// every broadcast publish below explicitly targets `write_relay_urls`
    /// (the main relays only), so the indexer never receives — and therefore
    /// never rejects — NIP-29 group events.
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
        // Full default flags (READ+WRITE+PING): READ so kind:0 lookups still
        // resolve here, WRITE so the explicit publish_event_to below can reach
        // it — `send_event_to` enforces the per-relay WRITE flag even when the
        // caller names the relay explicitly, so a READ-only add would make
        // every indexer publish fail with `write actions are disabled`.
        if let Some(url) = indexer_url {
            if !url.is_empty() {
                client
                    .add_relay(url)
                    .await
                    .with_context(|| format!("adding indexer relay {url}"))?;
            }
        }
        // Kick off the connection in the BACKGROUND (non-blocking) and return
        // immediately. Awaiting connectivity + NIP-42 auth is `warmup()`'s job,
        // which the daemon runs OFF its startup critical path so store-only RPCs
        // (`who`, hosted sessions, chat/inbox reads) serve instantly even when the relay
        // is slow or unreachable.
        client.connect().await;
        Ok(Self {
            client,
            pubkey,
            write_relay_urls: relays.to_vec(),
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

    /// Sign (with the connection's key) and publish an event template to the
    /// main relays (see [`Transport::write_relay_urls`] doc on why this can't
    /// just call the pool's implicit broadcast).
    pub async fn publish_builder(&self, builder: EventBuilder) -> Result<EventId> {
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_builder_to(self.write_relay_urls.iter().cloned(), builder)
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
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_builder_to(self.write_relay_urls.iter().cloned(), builder)
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
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_to(self.write_relay_urls.iter().cloned(), &signed)
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
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_to(self.write_relay_urls.iter().cloned(), &signed)
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
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_to(self.write_relay_urls.iter().cloned(), signed)
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
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_to(self.write_relay_urls.iter().cloned(), signed)
            .await
            .context("publishing signed event")?;
        assert_relay_accepted(&out, Some(signed))?;
        Ok(out.val)
    }

    /// Publish an already-signed event to a specific relay subset (by URL).
    /// Used by the indexer publish path: kind:0 profiles go to BOTH the
    /// indexer relay AND the main NIP-29 relay(s) — the main relay accepts
    /// kind:0 fine, so profiles must land there too (agent/backend name
    /// resolution shouldn't depend on the indexer alone). Falls back to the
    /// main relays when no explicit targets are given (preserves behavior
    /// for single-relay dev setups with no indexer configured).
    pub async fn publish_event_to(&self, signed: &Event, urls: &[String]) -> Result<EventId> {
        crate::relay_log::log_outgoing_event(signed);
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        if urls.is_empty() {
            let out = self
                .client
                .send_event_to(self.write_relay_urls.iter().cloned(), signed)
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

    /// The main NIP-29 relay URL(s) (see the [`Transport`] field doc).
    pub fn write_relay_urls(&self) -> &[String] {
        &self.write_relay_urls
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
    /// the live set at the actual working set (channels × agents × kinds).
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
