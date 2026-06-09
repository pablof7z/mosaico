//! NIP-29 group materializer: handles kind:39000 and kind:39002 relay events.
//!
//! These are relay-authored state events; materialize them into the local store
//! without touching the tail channel or mention routing.

use crate::state::Store;
use nostr_sdk::Event;

pub struct Nip29Materializer;

impl Nip29Materializer {
    /// Materialise kind:39000 — NIP-29 group metadata.
    ///
    /// Reads the `d` (project slug) and `about` tags and upserts the project
    /// metadata record using the event's creation timestamp. Byte-identical to
    /// the 39000 branch in `handle_incoming`.
    pub fn materialize_group_metadata(store: &Store, event: &Event) {
        if let (Some(project), about) = (
            super::nostr_tag(event, "d"),
            super::nostr_tag(event, "about").unwrap_or(""),
        ) {
            store
                .upsert_project_meta(project, about, event.created_at.as_secs())
                .ok();
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
                                s.get(2)
                                    .cloned()
                                    .unwrap_or_else(|| "member".to_string()),
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
}
