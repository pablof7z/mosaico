//! Narrow direct-wire adapter over `nostr-sdk`.
//!
//! Speaks the narrow direct-wire operations NMP cannot currently express:
//! one-shot fetches and the connectivity probe. NMP owns durable publication,
//! signing, receipts, retries, and live acquisition.

use anyhow::{Context, Result};
use nostr_sdk::prelude::*;
use std::time::Duration;

pub(crate) struct Transport {
    client: Client,
    /// Main relay targets for the explicit doctor probe. Product writes never
    /// use this direct client.
    write_relay_urls: Vec<String>,
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
fn assert_relay_accepted(output: &Output<EventId>) -> Result<()> {
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
        crate::relay_log::log_relay_rejection("no relay returned OK (timeout)", None);
        anyhow::bail!("no relay accepted the event (timeout or no OK received)");
    }
    let msg = reasons.join("; ");
    crate::relay_log::log_relay_rejection(&msg, None);
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
    /// Connect to the configured main relays plus an optional profile indexer.
    /// Product writes use NMP; this client only reads and runs doctor probes.
    pub(crate) async fn connect_with_indexer(
        relays: &[String],
        indexer_url: Option<&str>,
        keys: Keys,
    ) -> Result<Self> {
        let opts = ClientOptions::default().automatic_authentication(true);
        let client = Client::builder().signer(keys).opts(opts).build();
        for r in relays {
            client
                .add_relay(r)
                .await
                .with_context(|| format!("adding relay {r}"))?;
        }
        // The indexer participates in direct kind:0 lookups only. Product
        // profile writes are durable NMP pinned-host intents.
        if let Some(url) = indexer_url {
            if !url.is_empty() {
                client
                    .add_relay(url)
                    .await
                    .with_context(|| format!("adding indexer relay {url}"))?;
            }
        }
        // Initiate the connection in the BACKGROUND (not awaited here): the daemon
        // builds its Transport before the accept loop is spawned, and awaiting
        // `connect()` — which can block on the relay handshake under load — would
        // stall even store-only RPCs. `warmup()` awaits real connectivity + AUTH.
        let cc = client.clone();
        tokio::spawn(async move { cc.connect().await });
        Ok(Self {
            client,
            write_relay_urls: relays.to_vec(),
        })
    }

    /// Block (bounded) until the direct provider connection is established and
    /// complete any relay-requested AUTH before startup publishes or one-shot
    /// fetches. NMP's live observations use their own public access context.
    pub(crate) async fn warmup(&self) {
        self.client
            .wait_for_connection(Duration::from_secs(8))
            .await;
        let warmup = Filter::new().kind(Kind::from(0u16)).limit(1);
        let _ = self
            .client
            .fetch_events(warmup, Duration::from_secs(5))
            .await;
    }

    /// Publish the doctor's uniquely-tagged probe to the configured app relays.
    pub(crate) async fn publish_probe_checked(&self, marker: &str) -> Result<EventId> {
        let builder = doctor_probe_builder(marker)?;
        self.client.wait_for_connection(PUBLISH_CONNECT_WAIT).await;
        let out = self
            .client
            .send_event_builder_to(self.write_relay_urls.iter().cloned(), builder)
            .await
            .context("publishing event")?;
        assert_relay_accepted(&out)?;
        Ok(out.val)
    }

    /// One-shot query (used for resolution — e.g. fetch a `kind:0` profile).
    pub(crate) async fn fetch(&self, filter: Filter, timeout: Duration) -> Result<Vec<Event>> {
        let events = self
            .client
            .fetch_events(filter, timeout)
            .await
            .context("fetching events")?;
        Ok(events.into_iter().collect())
    }

    pub(crate) async fn shutdown(&self) {
        self.client.disconnect().await;
    }
}

fn doctor_probe_builder(marker: &str) -> Result<EventBuilder> {
    // `h` is a NIP-29 group id on the workshop relay. A random doctor marker
    // in `h` is rejected as a nonexistent group before it can prove transport
    // health. Use the ordinary topic tag instead.
    Ok(
        EventBuilder::new(Kind::from(1u16), format!("mosaico doctor {marker}"))
            .tags([Tag::parse(["t", marker])?]),
    )
}

#[cfg(test)]
mod tests;
