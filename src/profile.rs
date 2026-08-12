//! Single source of truth for current `kind:0` display-name resolution.
//!
//! Anything that needs a human-readable label for a pubkey — chat-mention
//! rendering, `who`, channel context — resolves it HERE so the policy lives in
//! one place:
//!
//!   1. A Row already owned by a retained NMP observation answers immediately.
//!   2. Otherwise a bounded exact-author NMP read returns its decoded Row value
//!      directly to the caller. It does not mutate Mosaico's retained view.
//!
//! Resolution is the reason remote agents and human operators show up by name
//! instead of a raw pubkey: their slug never rides the wire, so the only way to
//! learn it is their `kind:0`.

use crate::daemon::server::DaemonState;
use crate::state::Store;
use crate::util::pubkey_short;
use nostr::nips::nip19::Nip19Profile;
use nostr::{FromBech32, PublicKey};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Resolve `pubkey` from a retained Row or one bounded exact-author NMP read.
pub async fn resolve_name(state: &Arc<DaemonState>, pubkey: &str) -> Option<String> {
    if let Some(name) = state
        .with_store(|store| store.get_profile(pubkey).ok().flatten())
        .and_then(|profile| (!profile.name.is_empty()).then_some(profile.name))
    {
        return Some(name);
    }

    state
        .fabric_provider()
        .fetch_profile(pubkey)
        .await
        .map(|profile| profile.agent.slug)
        .filter(|name| !name.trim().is_empty())
}

/// Resolve a batch without relying on a write-through presentation cache.
pub async fn resolve_names(
    state: &Arc<DaemonState>,
    pubkeys: &[String],
) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    for pk in pubkeys {
        if let Some(name) = resolve_name(state, pk).await {
            names.insert(pk.clone(), name);
        }
    }
    names
}

/// Resolve display names for a batch of chat rows so the current render can use
/// names, not raw pubkeys, in two places:
///   - the **sender** label (`from_slug`), for rows whose author we never named
///     (a human operator or unseen remote agent), and
///   - every `nostr:npub1…` / `nostr:nprofile1…` mention **inside the body**.
///
/// Every referenced pubkey is resolved once. Bounded-read results are returned
/// for sender rendering and carried into body rewriting instead of relying on a
/// write-through side effect.
pub async fn resolve_chat_labels(
    state: &Arc<DaemonState>,
    rows: &mut [crate::state::InboxRow],
) -> BTreeMap<String, String> {
    let mut pubkeys: Vec<String> = Vec::new();
    for row in rows.iter() {
        pubkeys.push(row.from_pubkey.clone());
        pubkeys.extend(body_mention_pubkeys(&row.body));
    }
    pubkeys.sort();
    pubkeys.dedup();
    let names = resolve_names(state, &pubkeys).await;

    state.with_store(|s| {
        for row in rows.iter_mut() {
            row.body = rewrite_body_mentions_with_names(s, &row.body, &names);
        }
    });
    names
}

/// Replace every `nostr:npub1…` / `nostr:nprofile1…` mention in `text` with
/// `@<name>` from the retained NMP view. An unresolved pubkey falls back to a
/// short hex form so the output is never a wall of bech32.
pub fn rewrite_body_mentions(store: &Store, text: &str) -> String {
    rewrite_body_mentions_with_names(store, text, &BTreeMap::new())
}

fn rewrite_body_mentions_with_names(
    store: &Store,
    text: &str,
    fetched_names: &BTreeMap<String, String>,
) -> String {
    let mut out = text.to_string();
    for (token, entity) in nostr_entities(text) {
        let Some(pubkey) = decode_entity_pubkey(&entity) else {
            continue;
        };
        let label = fetched_names
            .get(&pubkey)
            .cloned()
            .or_else(|| {
                store
                    .get_profile(&pubkey)
                    .ok()
                    .flatten()
                    .and_then(|profile| (!profile.name.is_empty()).then_some(profile.name))
            })
            .unwrap_or_else(|| pubkey_short(&pubkey));
        out = out.replace(&token, &format!("@{label}"));
    }
    out
}

/// Hex pubkeys referenced by `nostr:` entity mentions in `text`.
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
