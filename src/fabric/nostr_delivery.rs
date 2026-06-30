//! Raw Nostr delivery: subscribes to relay streams for a given `Scope`.
//!
//! `Transport` is the private implementation detail; this struct is the
//! fabric-layer boundary that `resubscribe` in the daemon talks to.

use crate::fabric::nip29::wire::{
    h_filter, kind, KIND_CHAT, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA,
    KIND_PROFILE, KIND_STATUS,
};
use crate::fabric::{Delivery, Scope};
use crate::transport::Transport;
use anyhow::Result;
use nostr_sdk::prelude::*;
use std::sync::Arc;

pub struct NostrDelivery {
    transport: Arc<Transport>,
}

impl NostrDelivery {
    pub fn new(transport: Arc<Transport>) -> Self {
        Self { transport }
    }

    pub async fn subscribe(&self, scope: Scope) -> Result<()> {
        self.transport.subscribe(scope_filters(&scope)).await
    }
}

impl Delivery for NostrDelivery {
    fn name(&self) -> &'static str {
        "nostr"
    }
}

/// Build relay subscription filters for a given `Scope`.
///
/// The test `filters_cover_all_kinds_and_mentions` directly exercises this
/// function so subscription drift is caught at the delivery seam.
pub fn scope_filters(scope: &Scope) -> Vec<Filter> {
    let authors: Vec<PublicKey> = scope
        .authors
        .iter()
        .filter_map(|h| match PublicKey::from_hex(h) {
            Ok(pk) => Some(pk),
            Err(e) => {
                tracing::warn!(
                    author = h.as_str(),
                    error = %e,
                    "scope_filters: dropping malformed author pubkey hex — subscription coverage thinned"
                );
                None
            }
        })
        .collect();

    let with_authors = |mut f: Filter| -> Filter {
        if !authors.is_empty() {
            f = f.authors(authors.clone());
        }
        f
    };

    let mut filters = Vec::new();

    // Profiles (kind:0) — identity resolution.
    filters.push(with_authors(Filter::new().kind(kind(KIND_PROFILE))));

    // Presence + status (kind:30315) — live sessions and current work.
    let mut presence_status = Filter::new().kind(kind(KIND_STATUS));
    if let Some(p) = &scope.project {
        presence_status = h_filter(presence_status, p);
    }
    // Group-scoped events are not author-gated locally; the relay enforces
    // membership for groups this daemon owns (created closed via tenexPrivateKey).
    filters.push(presence_status);

    // Chat (kind:9) — NIP-29 group chat (the sole agent-to-agent channel).
    let mut chat = Filter::new().kind(kind(KIND_CHAT));
    if let Some(p) = &scope.project {
        chat = h_filter(chat, p);
    }
    filters.push(chat);

    // NIP-29 relay-authored group state (metadata/admins/members) for the
    // scoped group. Keeping this live is "check which groups we own at all
    // times": it feeds the membership cache. Addressable + relay-signed, so
    // filter by the `d` tag (group id == project slug), never by author.
    if let Some(p) = &scope.project {
        filters.push(
            Filter::new()
                .kinds([
                    kind(KIND_GROUP_METADATA),
                    kind(KIND_GROUP_ADMINS),
                    kind(KIND_GROUP_MEMBERS),
                ])
                .identifier(p),
        );
    }

    filters
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::Keys;

    #[test]
    fn filters_cover_all_kinds_and_mentions() {
        let me = Keys::generate().public_key().to_hex();
        let scope = crate::fabric::Scope {
            authors: vec![Keys::generate().public_key().to_hex()],
            project: Some("tenex-edge".into()),
            mentions_to: Some(me),
            owners: vec![Keys::generate().public_key().to_hex()],
        };
        let filters = scope_filters(&scope);
        // profiles, presence/status, chat (kind:9), and NIP-29
        // group-state (39000/39001/39002 by #d).
        assert_eq!(filters.len(), 4);
        let json = serde_json::to_string(&filters).unwrap();
        assert!(json.contains("\"#h\""));
        assert!(!json.contains("\"#t\""));
        // group-state filter present: addressable kinds scoped by #d=slug.
        assert!(json.contains("\"#d\""));
        assert!(json.contains("39002"));
    }
}
