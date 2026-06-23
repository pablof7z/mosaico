//! NIP-29 group materializer: handles kind:39000 and kind:39002 relay events.
//!
//! These are relay-authored state events; materialize them into the local store
//! without touching the tail channel or mention routing.

use crate::domain::ChatMessage;
use crate::state::{ChatInboxRow, ChatLogRow, Store};
use nostr_sdk::Event;

pub struct Nip29Materializer;

impl Nip29Materializer {
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
                            (
                                pk.clone(),
                                s.get(2).cloned().unwrap_or_else(|| "member".to_string()),
                            )
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
        // Chat (kind:9) carries from_session / mentioned_session as display
        // metadata tags. Mention (kind:1) routing fully cut over to the session
        // pubkey, but chat fans out to all alive project sessions, so these tags
        // remain the source of truth for "who sent / who was @-mentioned" — and
        // they're the only way to recover that in unmanaged (no-tenexPrivateKey)
        // mode where no session keys exist. Fall back to the session_pubkeys mapping
        // when a tag is absent (older publishers / managed-mode enrichment).
        let from_session = chat.from_session.clone().unwrap_or_else(|| {
            store
                .session_pubkey_info(&from_pubkey)
                .map(|(sid, _, _)| sid)
                .unwrap_or_default()
        });
        let mentioned_session = chat.mentioned_session.clone().unwrap_or_else(|| {
            chat.mentioned_pubkey
                .as_deref()
                .and_then(|pk| store.session_pubkey_info(pk).map(|(sid, _, _)| sid))
                .unwrap_or_default()
        });
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
            if rec.project != chat.project {
                continue;
            }
            if rec.created_at > created_at {
                continue;
            }
            if !from_session.is_empty() && rec.session_id == from_session {
                continue;
            }
            let row = ChatInboxRow {
                chat_event_id: event.id.to_hex(),
                target_session: rec.session_id,
                from_pubkey: from_pubkey.clone(),
                from_slug: from_slug.clone(),
                project: chat.project.clone(),
                body: chat.body.clone(),
                created_at,
                from_session: from_session.clone(),
                mentioned_session: mentioned_session.clone(),
            };
            if store.enqueue_chat(&row).unwrap_or(false) {
                routed = true;
            }
        }
        routed
    }
}
