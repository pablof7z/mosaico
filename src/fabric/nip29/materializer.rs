//! NIP-29 inbound materializer.
//!
//! The single intake point for every relay event the daemon observes. Each event
//! is routed by kind into exactly one of the `relay_*` caches (channels, members,
//! profiles, status) or, for every other kind, the verbatim `relay_events` log.
//! Chat (kind:9) is additionally routed into the `inbox` delivery ledger for
//! local sessions occupying the event's channel. The same ledger stores
//! per-target orchestration claims with synthetic target keys.
//!
//! None of these writes touch authoritative local truth: `relay_*` are caches,
//! identical for local and remote agents, rebuildable from the relay at any time.

use crate::domain::{ChatMessage, Profile};
use crate::state::{RelayEvent, Store};
use nostr_sdk::Event;

mod agent_roster;
mod messages;

pub struct Nip29Materializer;

impl Nip29Materializer {
    // ── relay_channels (kind:39000) ──────────────────────────────────────────

    /// Materialise kind:39000 group metadata into `relay_channels`. The group id
    /// is the event's `d` tag; `parent` (empty for top-level root channels)
    /// distinguishes a session/task channel from a root channel.
    pub fn materialize_channel(store: &Store, event: &Event) {
        let Some(channel_h) = super::nostr_tag(event, "d") else {
            return;
        };
        let name = super::nostr_tag(event, "name").unwrap_or("");
        let about = super::nostr_tag(event, "about").unwrap_or("");
        let parent = super::nostr_tag(event, "parent").unwrap_or("");
        if let Err(e) =
            store.upsert_channel(channel_h, name, about, parent, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_channel: relay_channels upsert failed — relay truth diverged from cache"
            );
        }
    }

    // ── relay_channel_members (kind:39001 admins / 39002 members) ─────────────

    /// Materialise kind:39001 — replace the admin rows for the channel, preserving
    /// member rows.
    pub fn materialize_admins(store: &Store, event: &Event) {
        let Some(channel_h) = super::nostr_tag(event, "d") else {
            return;
        };
        let admins = collect_p_pubkeys(event);
        if let Err(e) = store.replace_channel_admins(channel_h, &admins, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_admins: replace_channel_admins failed — relay truth diverged from cache"
            );
        }
    }

    /// Materialise kind:39002 — replace the member rows for the channel, preserving
    /// admin rows.
    pub fn materialize_members(store: &Store, event: &Event) {
        let Some(channel_h) = super::nostr_tag(event, "d") else {
            return;
        };
        let members = collect_p_pubkeys(event);
        if let Err(e) =
            store.replace_channel_members(channel_h, &members, event.created_at.as_secs())
        {
            tracing::error!(
                channel = channel_h,
                error = %e,
                "materialize_members: replace_channel_members failed — relay truth diverged from cache"
            );
        }
    }

    // ── relay_profiles (kind:0) ──────────────────────────────────────────────

    /// Materialise a decoded kind:0 profile into `relay_profiles`. Newer
    /// `updated_at` wins. Agent profile `name`/`slug` are the canonical
    /// `agent/session` handle; backend profiles keep their backend name.
    pub fn materialize_profile(store: &Store, pf: &Profile, updated_at: u64) {
        let slug = pf.agent.slug.as_str();
        let name = if pf.is_backend {
            slug.to_string()
        } else {
            crate::idref::session_handle_from_profile_name(slug, &pf.host, &pf.agent_slug)
        };
        let slug = if pf.is_backend {
            slug.to_string()
        } else {
            name.clone()
        };
        if let Err(e) = store.upsert_profile_with_agent_slug(
            &pf.agent.pubkey,
            &name,
            &slug,
            &pf.agent_slug,
            &pf.host,
            pf.is_backend,
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
    /// `(pubkey, session_id, channel_h)`. A single status event may carry several
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
        for channel in &st.channels {
            if let Err(e) = store.upsert_status(&crate::state::Status {
                pubkey: st.agent.pubkey.clone(),
                session_id: st.session_id.as_str().to_string(),
                channel_h: channel.clone(),
                slug: slug.clone(),
                title: st.title.clone(),
                activity: st.activity.clone(),
                busy: st.busy,
                last_seen: updated_at,
                updated_at,
                expiration: st.expires_at.unwrap_or(0),
            }) {
                tracing::error!(
                    pubkey = %st.agent.pubkey,
                    session = %st.session_id,
                    channel,
                    error = %e,
                    "materialize_status: relay_status upsert failed — relay truth diverged from cache"
                );
            }
        }
    }

    pub fn materialize_agent_roster(store: &Store, event: &Event) {
        agent_roster::materialize(store, event);
    }

    // ── relay_events (every other kind, verbatim) ────────────────────────────

    /// Cache one relay event verbatim in `relay_events` (NIP-01 replacement is
    /// applied inside the store). Used for every kind that has no dedicated cache:
    /// chat (9), notes/activity (1), proposals (30023), orchestration, etc.
    pub fn materialize_event(store: &Store, event: &Event) -> bool {
        store.insert_event(&to_relay_event(event)).unwrap_or(false)
    }

    /// Route a chat message into the `inbox` ledger for every alive local session
    /// whose agent is explicitly p-tagged in the event. Non-mention channel chat
    /// stays in `relay_events` for ambient context but does not ring the direct
    /// doorbell. Returns `true` if at least one new inbox row was enqueued.
    /// Idempotent: a duplicate `(event_id, target_session)` is ignored by the store.
    pub fn route_chat(store: &Store, event: &Event, chat: &ChatMessage) -> bool {
        let channel_h = chat.channel.as_str();
        let from_pubkey = event.pubkey.to_hex();
        let event_id = event.id.to_hex();
        let created_at = event.created_at.as_secs();
        let p_pubkeys = collect_p_pubkeys(event);
        if p_pubkeys.is_empty() {
            return false;
        }
        // A DB error reading the live-session set must NOT be silently collapsed
        // into "zero sessions" — that would drop the mention and never wake the
        // agent. Fail loud; the chat row is still cached in relay_events.
        let sessions = match store.list_alive_sessions() {
            Ok(sessions) => sessions,
            Err(e) => {
                tracing::error!(
                    channel = channel_h,
                    event_id = %event_id,
                    error = %e,
                    "route_chat: list_alive_sessions failed — mention not routed to any session (possible message loss)"
                );
                return false;
            }
        };
        let mut woke = false;
        for sess in sessions {
            if sess.agent_pubkey == from_pubkey {
                continue;
            }
            let joined = store
                .is_session_joined_channel(&sess.session_id, channel_h)
                .unwrap_or(sess.channel_h == channel_h);
            if !joined {
                continue;
            }
            if !p_pubkeys.contains(&sess.agent_pubkey) {
                continue;
            }
            match store.enqueue_inbox(
                &event_id,
                &sess.session_id,
                &from_pubkey,
                channel_h,
                &chat.body,
                created_at,
            ) {
                Ok(true) => woke = true,
                Ok(false) => {}
                // A matched session whose inbox write failed is a dropped mention:
                // the agent will never see it. Surface loudly rather than folding
                // the failure into woke=false.
                Err(e) => tracing::error!(
                    session = %sess.session_id,
                    channel = channel_h,
                    event_id = %event_id,
                    error = %e,
                    "route_chat: enqueue_inbox failed for matched session — mention not delivered (agent not woken)"
                ),
            }
            if let Err(e) = store.add_message_recipient(
                &event_id,
                &sess.agent_pubkey,
                Some(&sess.session_id),
                None,
            ) {
                tracing::error!(
                    session = %sess.session_id,
                    channel = channel_h,
                    event_id = %event_id,
                    error = %e,
                    "route_chat: recipient session edge upsert failed"
                );
            }
        }
        woke
    }
}

/// All `p`-tag pubkey values (`slice[1]`) on the event.
fn collect_p_pubkeys(event: &Event) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) == Some("p") {
                s.get(1).cloned()
            } else {
                None
            }
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
