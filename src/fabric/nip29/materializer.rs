//! NIP-29 group materializer: handles kind:39000 and kind:39002 relay events.
//!
//! These are relay-authored state events; materialize them into the local store
//! without touching the tail channel or mention routing.

use crate::domain::{ChatMessage, Profile, Status};
use crate::session::PeerStatusObservation;
use crate::state::{ChatInboxRow, ChatLogRow, Store};
use nostr_sdk::Event;

pub struct Nip29Materializer;

impl Nip29Materializer {
    /// Apply a decoded `Profile` (kind:0) to the store.
    ///
    /// NIP-29 admission happens at the relay/group layer. If a profile event is
    /// delivered by our scoped subscription, persist it for identity resolution.
    pub fn materialize_profile(store: &Store, pf: &Profile, now: u64) {
        let pk = &pf.agent.pubkey;
        store.upsert_profile(pk, &pf.agent.slug, &pf.host, now).ok();
    }

    /// Apply a decoded peer `Status` (kind:30315) to `peer_session_state`.
    ///
    /// Local sessions live in `session_state`, written exclusively by daemon
    /// transitions. This path only mirrors peer status observed on the relay.
    pub fn materialize_status(store: &Store, st: &Status, seen_at: u64, now: u64) {
        if let Some(exp) = st.expires_at {
            if exp <= now {
                return;
            }
        }
        let slug = if !st.agent.slug.is_empty() {
            st.agent.slug.clone()
        } else {
            store
                .resolve_slug_for_pubkey(&st.agent.pubkey)
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        store
            .record_peer_status(&PeerStatusObservation {
                agent_pubkey: st.agent.pubkey.clone(),
                agent_slug: slug,
                project: st.project.clone(),
                host: st.host.clone(),
                rel_cwd: st.rel_cwd.clone(),
                title: st.title.clone(),
                activity: st.activity.clone(),
                busy: st.busy,
                emitted_at: seen_at,
                observed_at: now,
            })
            .ok();
    }

    /// Materialise kind:39000 — NIP-29 group metadata.
    ///
    /// Reads the `d` (project slug) and `about` tags and upserts the project
    /// metadata record using the event's creation timestamp. Byte-identical to
    /// the 39000 branch in `handle_incoming`.
    pub fn materialize_group_metadata(store: &Store, event: &Event) {
        if let Some(project) = super::nostr_tag(event, "d") {
            let ts = event.created_at.as_secs();
            let about = super::nostr_tag(event, "about").unwrap_or("");
            store.upsert_project_meta(project, about, ts).ok();
            // NIP-29 subgroup hierarchy (issue #3): capture the display name and
            // the parent relationship so `groups list` can render the tree from
            // local state. `parent` is empty for top-level project groups.
            let name = super::nostr_tag(event, "name").unwrap_or("");
            let parent = super::nostr_tag(event, "parent").unwrap_or("");
            if !name.is_empty() || !parent.is_empty() {
                store.upsert_group_metadata(project, name, parent, ts).ok();
            }
        }
    }

    /// Materialise kind:39002 — NIP-29 membership snapshot.
    ///
    /// Collects all `p` tags (pubkey + optional role, defaulting to "member")
    /// and replaces the group member set using the event's creation timestamp.
    /// Byte-identical to the 39002 branch in `handle_incoming`.
    pub fn materialize_membership_snapshot(store: &Store, event: &Event) {
        if let Some(project) = super::nostr_tag(event, "d") {
            let members: Vec<(String, String)> = event
                .tags
                .iter()
                .filter_map(|t| {
                    let s = t.as_slice();
                    if s.first().map(String::as_str) == Some("p") {
                        s.get(1).map(|pk| {
                            let role = s.get(2).cloned().unwrap_or_else(|| "member".to_string());
                            eprintln!(
                                "[daemon] nip29-role-decision group={project} target={} role={} reason=materialize relay 39002 membership snapshot",
                                crate::util::pubkey_short(pk),
                                role
                            );
                            (pk.clone(), role)
                        })
                    } else {
                        None
                    }
                })
                .collect();
            store
                .replace_group_members(project, &members, event.created_at.as_secs())
                .ok();
        }
    }

    /// Route one NIP-29 group chat message into the live chat queue for sessions
    /// that were already alive when the event was created. This is deliberately
    /// not a historical catch-up path: sessions started after the chat line was
    /// published do not receive it.
    pub fn materialize_chat_message(store: &Store, chat: &ChatMessage, event: &Event) -> bool {
        let created_at = event.created_at.as_secs();
        let from_pubkey = event.pubkey.to_hex();
        let from_slug = if chat.from.slug.is_empty() {
            store
                .resolve_slug_for_pubkey(&from_pubkey)
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            chat.from.slug.clone()
        };
        let from_session = store
            .session_pubkey_info(&from_pubkey)
            .map(|(sid, _, _)| sid)
            .unwrap_or_default();
        let mentioned_session = chat
            .mentioned_pubkey
            .as_deref()
            .and_then(|pk| store.session_pubkey_info(pk).map(|(sid, _, _)| sid))
            .unwrap_or_default();
        let host = store
            .resolve_chat_host(
                &from_pubkey,
                if from_session.is_empty() {
                    None
                } else {
                    Some(from_session.as_str())
                },
            )
            .ok()
            .flatten()
            .unwrap_or_default();

        let _ = store.record_chat(&ChatLogRow {
            chat_event_id: event.id.to_hex(),
            from_pubkey: from_pubkey.clone(),
            from_slug: from_slug.clone(),
            host,
            project: chat.project.clone(),
            body: chat.body.clone(),
            created_at,
            from_session: from_session.clone(),
            mentioned_session: mentioned_session.clone(),
        });

        let mut routed = false;
        for rec in store.list_alive_sessions().unwrap_or_default() {
            // Match on the session's CURRENT routing scope (channel when set,
            // else the per-session room) so a `channels switch` is reflected
            // immediately for inbound chat — otherwise a switched session keeps
            // receiving chat in its old room and misses everything published to
            // the channel it moved to.
            if rec.route_scope() != chat.project {
                continue;
            }
            if rec.created_at > created_at {
                continue;
            }
            // Skip sender's own session by session pubkey when available, then
            // durable pubkey for emitters that still sign chat with durable keys.
            if (!from_session.is_empty() && rec.session_id == from_session)
                || rec.agent_pubkey == from_pubkey
            {
                continue;
            }
            // A `p` tag may carry either a session pubkey or a durable agent
            // pubkey. Resolve session pubkeys through the local map, and keep a
            // durable-pubkey fallback for agent-level mentions.
            let row_mentioned = if mentioned_session == rec.session_id
                || chat.mentioned_pubkey.as_deref() == Some(rec.agent_pubkey.as_str())
            {
                rec.session_id.clone()
            } else {
                String::new()
            };
            let row = ChatInboxRow {
                chat_event_id: event.id.to_hex(),
                target_session: rec.session_id,
                from_pubkey: from_pubkey.clone(),
                from_slug: from_slug.clone(),
                project: chat.project.clone(),
                body: chat.body.clone(),
                created_at,
                from_session: from_session.clone(),
                mentioned_session: row_mentioned,
            };
            if store.enqueue_chat(&row).unwrap_or(false) {
                routed = true;
            }
        }
        routed
    }
}
