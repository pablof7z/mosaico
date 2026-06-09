//! Raw Nostr delivery: subscribes to relay streams for a given `Scope`.
//!
//! `Transport` is the private implementation detail; this struct is the
//! fabric-layer boundary that `resubscribe` in the daemon talks to.

use crate::codec::kind1::{
    h_filter, kind, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA, KIND_NOTE,
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
/// This is the EXACT logic relocated verbatim from `Kind1Codec::filters`
/// (src/codec/kind1.rs:309-376). `Kind1Codec::filters` now delegates to this
/// function (via `Scope::from(&scope)`), so the test
/// `filters_cover_all_kinds_and_mentions` transitively verifies this path.
pub fn scope_filters(scope: &Scope) -> Vec<Filter> {
    let authors: Vec<PublicKey> = scope
        .authors
        .iter()
        .filter_map(|h| PublicKey::from_hex(h).ok())
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
    // membership for groups this daemon owns (created closed via userNsec).
    filters.push(presence_status);

    // Notes (kind:1) — activity + mentions.
    let mut notes = Filter::new().kind(kind(KIND_NOTE));
    if let Some(p) = &scope.project {
        notes = h_filter(notes, p);
    }
    filters.push(notes);

    // Mentions addressed to me (may arrive without a project group match).
    if let Some(me) = &scope.mentions_to {
        if let Ok(pk) = PublicKey::from_hex(me) {
            filters.push(Filter::new().kind(kind(KIND_NOTE)).pubkey(pk));
        }
    }

    // Discover ANY profile (any author) that claims one of our owners — the
    // ACL pending set. Deliberately NOT author-restricted.
    for owner in &scope.owners {
        if let Ok(pk) = PublicKey::from_hex(owner) {
            filters.push(Filter::new().kind(kind(KIND_PROFILE)).pubkey(pk));
        }
    }

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
