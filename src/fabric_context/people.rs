use super::model::{MemberRow, PresenceRow};
use super::refs::pubkey_ref;
use super::FabricContextInput;
use crate::state::{Status, Store};
use crate::util::relative_time;
use std::collections::BTreeMap;

pub(super) fn member_rows(
    store: &Store,
    channel: &str,
    input: &FabricContextInput<'_>,
) -> Vec<MemberRow> {
    let statuses = status_map(store, channel, input.now);
    // Keyed by pubkey so iteration order matches the pure `assemble` path (which
    // reads the same relay roster into a `BTreeMap<pubkey, role>`).
    let roster = store
        .list_channel_members(channel)
        .unwrap_or_default()
        .into_iter()
        .map(|m| (m.pubkey, m.role))
        .collect::<BTreeMap<_, _>>();
    roster
        .into_iter()
        // Exclude this daemon's own management key by identity (reliable on a cold
        // cache), plus any pubkey whose cached kind:0 declares itself a backend
        // (covers remote backends). Human operators and admins are kept.
        .filter(|(pk, _)| pk.as_str() != input.backend_pubkey && !is_backend(store, pk))
        .map(|(pk, _role)| {
            let status = statuses.get(&pk);
            let status_text = status
                .map(status_text)
                .unwrap_or_else(|| "offline".to_string());
            let seen = status
                .map(|s| relative_time(s.last_seen, input.now))
                .unwrap_or_else(|| "unknown".to_string());
            let reference = if pk == input.self_pubkey {
                crate::idref::agent_ref_from(input.self_slug, input.local_host, input.local_host)
            } else {
                member_reference(store, &pk, status, input.local_host)
            };
            MemberRow {
                reference,
                status: status_text,
                seen,
            }
        })
        .collect()
}

/// A non-self member's reference: the live status slug when known, else the
/// profile/npub fallback.
/// Mirrors `assemble::member_reference` exactly so both paths agree.
fn member_reference(store: &Store, pk: &str, status: Option<&Status>, local_host: &str) -> String {
    if let Some(slug) = status
        .map(|s| s.slug.trim())
        .filter(|slug| !slug.is_empty())
    {
        return slug.to_string();
    }
    pubkey_ref(store, pk, local_host)
}

pub(super) fn presence_rows(
    store: &Store,
    channel: &str,
    input: &FabricContextInput<'_>,
) -> Vec<PresenceRow> {
    if input.cursor == 0 {
        return Vec::new();
    }
    store
        .live_status_for_channel(channel, input.now)
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.updated_at > input.cursor)
        .filter(|s| s.pubkey != input.self_pubkey)
        .map(|s| PresenceRow {
            reference: presence_reference(store, &s, input.local_host),
            status: status_text(&s),
            seen: relative_time(s.last_seen, input.now),
        })
        .collect()
}

fn presence_reference(store: &Store, status: &Status, local_host: &str) -> String {
    if !status.slug.trim().is_empty() {
        return status.slug.clone();
    }
    pubkey_ref(store, &status.pubkey, local_host)
}

fn status_map(store: &Store, channel: &str, now: u64) -> BTreeMap<String, Status> {
    store
        .live_status_for_channel(channel, now)
        .unwrap_or_default()
        .into_iter()
        .map(|s| (s.pubkey.clone(), s))
        .collect()
}

fn status_text(status: &Status) -> String {
    if status.busy {
        return non_empty(&status.activity)
            .or_else(|| non_empty(&status.title))
            .unwrap_or_else(|| "working".to_string());
    }
    non_empty(&status.title).unwrap_or_else(|| "idle".to_string())
}

fn is_backend(store: &Store, pubkey: &str) -> bool {
    store
        .get_profile(pubkey)
        .ok()
        .flatten()
        .map(|p| p.is_backend)
        .unwrap_or(false)
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}
