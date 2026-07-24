//! Single source of truth for `kind:0` display-name resolution.
//!
//! Anything that needs a human-readable label for a pubkey — chat-mention
//! rendering, `who`, channel context — resolves it HERE so the policy lives in
//! one place:
//!
//!   1. **Cache.** The `profiles` table is the local cache (it is also written
//!      by the live demux when a `kind:0` arrives on a subscription). A fresh
//!      entry is returned without touching the network.
//!   2. **Relay fetch on miss/TTL.** A cache miss — or an entry older than
//!      [`PROFILE_TTL_SECS`] — triggers a bounded NMP `kind:0` observation,
//!      which is parsed and written back to the cache.
//!   3. **Stale fallback.** If the relay fetch fails (offline, timeout) but a
//!      stale cached name exists, that is returned rather than nothing.
//!
//! Resolution is the reason remote agents and human operators show up by name
//! instead of a raw pubkey: their slug never rides the wire, so the only way to
//! learn it is their `kind:0`.

use crate::daemon::server::DaemonState;
use crate::state::Store;
use crate::util::{now_secs, pubkey_short};
use nostr::nips::nip19::Nip19Profile;
use nostr::{FromBech32, PublicKey};
use std::sync::Arc;

/// How long a cached `kind:0` entry is trusted before a re-fetch. Profiles
/// change rarely, so a long window keeps relay traffic low; a stale name only
/// costs a slightly outdated label until the next refresh.
pub const PROFILE_TTL_SECS: u64 = 6 * 60 * 60;

/// Resolve `pubkey` to a display name, going to the relays on a cache miss or
/// TTL expiry. Returns `None` only when nothing is cached AND no `kind:0` could
/// be fetched.
pub async fn resolve_name(state: &Arc<DaemonState>, pubkey: &str) -> Option<String> {
    let now = now_secs();
    let cached = state.with_store(|s| s.get_profile(pubkey).ok().flatten());

    if let Some(p) = &cached {
        if !p.name.is_empty() && now.saturating_sub(p.updated_at) < PROFILE_TTL_SECS {
            return Some(p.name.clone());
        }
    }

    if let Some(name) = state
        .fabric_provider()
        .fetch_and_cache_profile_name(pubkey, now)
        .await
    {
        return Some(name);
    }

    // Relay miss/failure: fall back to whatever stale name we had.
    cached.map(|p| p.name).filter(|n| !n.is_empty())
}

/// Warm the cache for several pubkeys at once (e.g. every distinct sender of a
/// batch of pending chat rows) so the subsequent synchronous render resolves
/// each label from the cache. Results are discarded; the side effect is the
/// cache write.
pub async fn warm(state: &Arc<DaemonState>, pubkeys: &[String]) {
    for pk in pubkeys {
        let _ = resolve_name(state, pk).await;
    }
}

/// Resolve display names for a batch of chat rows so they render with names,
/// not raw pubkeys, in two places:
///   - the **sender** label (`from_slug`), for rows whose author we never named
///     (a human operator or unseen remote agent), and
///   - every `nostr:npub1…` / `nostr:nprofile1…` mention **inside the body**.
///
/// Every referenced pubkey is resolved once (cache→relay via [`warm`]); then the
/// labels and body rewrites are applied synchronously from the now-warm cache.
pub async fn label_chat_senders(state: &Arc<DaemonState>, rows: &mut [crate::state::InboxRow]) {
    let mut pubkeys: Vec<String> = Vec::new();
    for row in rows.iter() {
        pubkeys.push(row.from_pubkey.clone());
        pubkeys.extend(body_mention_pubkeys(&row.body));
    }
    pubkeys.sort();
    pubkeys.dedup();
    warm(state, &pubkeys).await;

    state.with_store(|s| {
        for row in rows.iter_mut() {
            row.body = rewrite_body_mentions(s, &row.body);
        }
    });
}

/// Replace every `nostr:npub1…` / `nostr:nprofile1…` mention in `text` with
/// `@<name>`, resolving each pubkey through the local profile cache (no network
/// — callers [`warm`] the cache first). An unresolved pubkey falls back to a
/// short hex form so the output is never a wall of bech32. This is the single
/// rendering of nostr entity mentions.
pub fn rewrite_body_mentions(store: &Store, text: &str) -> String {
    let mut out = text.to_string();
    for (token, entity) in nostr_entities(text) {
        let Some(pubkey) = decode_entity_pubkey(&entity) else {
            continue;
        };
        let label = store
            .get_profile(&pubkey)
            .ok()
            .flatten()
            .and_then(|p| (!p.name.is_empty()).then_some(p.name))
            .unwrap_or_else(|| pubkey_short(&pubkey));
        out = out.replace(&token, &format!("@{label}"));
    }
    out
}

/// Hex pubkeys referenced by `nostr:` entity mentions in `text` — the set the
/// caller must [`warm`] before [`rewrite_body_mentions`] can name them.
pub fn body_mention_pubkeys(text: &str) -> Vec<String> {
    nostr_entities(text)
        .into_iter()
        .filter_map(|(_, entity)| decode_entity_pubkey(&entity))
        .collect()
}

/// Scan `text` for `nostr:<bech32>` tokens, returning `(full_token, entity)`
/// pairs for npub/nprofile entities. The bech32 run is the contiguous lowercase
/// alphanumeric span after `nostr:` (bech32 is lowercase; the span stops at the
/// first space/punctuation), so a mention embedded in prose is captured cleanly.
fn nostr_entities(text: &str) -> Vec<(String, String)> {
    const PREFIX: &str = "nostr:";
    let mut out = Vec::new();
    let mut search_from = 0usize;
    while let Some(rel) = text[search_from..].find(PREFIX) {
        let entity_start = search_from + rel + PREFIX.len();
        let entity: String = text[entity_start..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .collect();
        // Advance past this match (at least one byte to guarantee progress).
        search_from = entity_start + entity.len().max(1);
        if entity.starts_with("npub1") || entity.starts_with("nprofile1") {
            out.push((format!("{PREFIX}{entity}"), entity));
        }
    }
    out
}

/// Decode a bech32 `npub`/`nprofile` entity to a hex pubkey.
fn decode_entity_pubkey(entity: &str) -> Option<String> {
    if let Ok(pk) = PublicKey::parse(entity) {
        return Some(pk.to_hex());
    }
    if let Ok(profile) = Nip19Profile::from_bech32(entity) {
        return Some(profile.public_key.to_hex());
    }
    None
}
