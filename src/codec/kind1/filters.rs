use super::*;

pub(super) fn filters(scope: &SubScope) -> Vec<Filter> {
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
