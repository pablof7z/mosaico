//! NIP-29 inbound materializer.
//!
//! The single intake point for every relay event the daemon observes. Each event
//! is routed by kind into exactly one of the `relay_*` caches (channels, members,
//! profiles, status) or, for every other kind, the verbatim `relay_events` log.
//! Direct-message execution is deliberately outside this transport projection:
//! the daemon classifies its own identities after the relay event is cached.
//!
//! None of these writes touch authoritative local truth: `relay_*` are caches,
//! identical for local and remote agents, rebuildable from the relay at any time.

use crate::domain::Profile;
use crate::state::{RelayEvent, Store};
use nostr::Event;

mod group_state;
mod messages;
mod reactions;

pub struct Nip29Materializer;

impl Nip29Materializer {
    // ── relay_profiles (kind:0) ──────────────────────────────────────────────

    /// Materialise a decoded kind:0 profile into `relay_profiles`. Newer
    /// `updated_at` wins. Agent profile `name`/`slug` are the canonical
    /// `sessionCode-agent` handle; backend profiles keep their backend name.
    pub fn materialize_profile(store: &Store, pf: &Profile, updated_at: u64) {
        let slug = pf.agent.slug.as_str();
        let name = if pf.is_backend {
            slug.to_string()
        } else {
            crate::idref::session_handle_from_profile_name(slug, &pf.agent_slug)
        };
        let slug = if pf.is_backend {
            slug.to_string()
        } else {
            name.clone()
        };
        if let Err(e) = store.upsert_profile_snapshot(
            &pf.agent.pubkey,
            &name,
            &slug,
            &pf.agent_slug,
            &pf.host,
            pf.is_backend,
            &pf.agents,
            &pf.workspaces,
            updated_at,
        ) {
            tracing::error!(
                pubkey = %pf.agent.pubkey,
                slug = %slug,
                error = %e,
                "materialize_profile: relay_profiles upsert failed — relay truth diverged from cache"
            );
        }
    }

    // ── relay_status (kind:30315) ────────────────────────────────────────────

    /// Materialise a decoded kind:30315 status into `relay_status`, one row per
    /// `(pubkey, channel_h)`. A single status event may carry several
    /// `h` tags; each becomes a channel row with the same session title/activity.
    /// Liveness is computed on READ from the NIP-40 `expiration`; the row is stored
    /// regardless of freshness (older `updated_at` writes are dropped by the store).
    pub fn materialize_status(store: &Store, st: &crate::domain::Status, updated_at: u64) {
        let slug = if !st.agent.slug.is_empty() {
            st.agent.slug.clone()
        } else {
            store
                .resolve_slug_for_pubkey(&st.agent.pubkey)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let statuses = st
            .channels
            .iter()
            .map(|channel| crate::state::Status {
                pubkey: st.agent.pubkey.clone(),
                channel_h: channel.clone(),
                slug: slug.clone(),
                title: st.title.clone(),
                activity: st.activity.clone(),
                workspace: st.workspace.clone(),
                branch: st.branch.clone(),
                state: st.state,
                state_since: st.state_since,
                last_seen: updated_at,
                updated_at,
                expiration: st.expires_at.unwrap_or(0),
            })
            .collect::<Vec<_>>();
        if let Err(e) = store.replace_status_channels(&st.agent.pubkey, &statuses, updated_at) {
            tracing::error!(
                pubkey = %st.agent.pubkey,
                error = %e,
                "materialize_status: relay_status snapshot replacement failed"
            );
        }
    }

    // ── relay_events (every other kind, verbatim) ────────────────────────────

    /// Cache one relay event verbatim in `relay_events` (NIP-01 replacement is
    /// applied inside the store). Used for every kind that has no dedicated cache:
    /// chat (9), notes/activity (1), orchestration, and other unprojected kinds.
    pub fn materialize_event(store: &Store, event: &Event) -> bool {
        store.insert_event(&to_relay_event(event)).unwrap_or(false)
    }
}

/// All `p`-tag pubkey values (`slice[1]`) on the event.
fn collect_p_pubkeys(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| {
            let parts = tag.as_slice();
            (parts.first().map(String::as_str) == Some("p"))
                .then(|| parts.get(1).cloned())
                .flatten()
        })
        .collect()
}

/// Channel a raw Nostr event onto the verbatim `relay_events` row shape.
pub(crate) fn to_relay_event(event: &Event) -> RelayEvent {
    RelayEvent {
        id: event.id.to_hex(),
        kind: event.kind.as_u16() as u32,
        pubkey: event.pubkey.to_hex(),
        created_at: event.created_at.as_secs(),
        channel_h: super::nostr_tag(event, "h").unwrap_or("").to_string(),
        d_tag: super::nostr_tag(event, "d").unwrap_or("").to_string(),
        content: event.content.clone(),
        tags_json: tags_to_json(event),
    }
}

/// Serialise the event tags as a JSON array of string arrays (NIP-01 shape).
fn tags_to_json(event: &Event) -> String {
    let raw: Vec<Vec<String>> = event.tags.iter().map(|t| t.as_slice().to_vec()).collect();
    serde_json::to_string(&raw).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests;
