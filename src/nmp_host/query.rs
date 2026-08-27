//! How a Mosaico question becomes a fava `Query`.
//!
//! Each `SubscriptionQuery` variant maps to a declarative `fava::Query`. The
//! relay scope is always explicit (`from_relays` / `only_from_relays`) so
//! that authority rules are stated once here and never inherited by accident.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use fava::{Query, RelayUrl, SingleLetterTag};

use crate::reconcile::SubscriptionQuery;

use super::NmpHost;

impl NmpHost {
    /// Convert one Mosaico subscription question into a fava `Query`.
    pub(crate) fn fava_query(&self, query: &SubscriptionQuery) -> Result<Query> {
        match query {
            SubscriptionQuery::GroupContentsMulti { groups, kinds } => {
                self.group_contents_multi_query(groups, kinds)
            }
            SubscriptionQuery::References { event_id, kinds } => {
                self.references_query(event_id, kinds)
            }
        }
    }

    /// Content kinds for SEVERAL groups at once, scoped by multi-value `#h`.
    fn group_contents_multi_query(
        &self,
        groups: &BTreeSet<String>,
        kinds: &BTreeSet<u16>,
    ) -> Result<Query> {
        let h = SingleLetterTag::from_char('h').expect("h is a single-letter tag");
        let mut q = Query::events();
        if !kinds.is_empty() {
            q = q
                .kinds(kinds.iter().map(|&k| fava::Kind::from_u16(k)))
                .context("group-contents-multi kinds")?;
        }
        q = q
            .tag_values(h, groups.iter().map(String::as_str))
            .context("group-contents-multi h-tag")?
            .only_from_relays(self.relays.iter().cloned())
            .context("group-contents-multi relay scope")?;
        Ok(q)
    }

    /// Events with `#e` tag = event_id, from the group relays.
    fn references_query(&self, event_id: &str, kinds: &BTreeSet<u16>) -> Result<Query> {
        let e = SingleLetterTag::from_char('e').expect("e is a single-letter tag");
        let mut q = Query::events();
        if !kinds.is_empty() {
            q = q
                .kinds(kinds.iter().map(|&k| fava::Kind::from_u16(k)))
                .context("references kinds")?;
        }
        q = q
            .tag_values(e, [event_id])
            .context("references e-tag")?
            .only_from_relays(self.relays.iter().cloned())
            .context("references relay scope")?;
        Ok(q)
    }

    /// kind:0 for one pubkey from the profile relays.
    fn profile_query(&self, pubkey: &str) -> Result<Query> {
        let pubkey = nostr::key::PublicKey::from_hex(pubkey)
            .with_context(|| format!("invalid pubkey in profile query: {pubkey}"))?;
        Query::events()
            .kinds([fava::Kind::from_u16(0)])
            .context("profile kinds")?
            .authors([pubkey])
            .context("profile author")?
            .from_relays(self.profile_relays.iter().cloned())
            .context("profile relay scope")
    }

    /// kind:0 for the current member set, from the profile relays.
    /// Used by the retained profile feed.
    pub(crate) fn profile_feed_query(&self, authors: &BTreeSet<String>) -> Result<Query> {
        if authors.is_empty() {
            // Empty author set matches nothing — return a vacuous query.
            return Ok(Query::events()
                .kinds([fava::Kind::from_u16(0)])
                .context("profile-feed kinds")?);
        }
        let pubkeys = authors
            .iter()
            .filter_map(|hex| nostr::key::PublicKey::from_hex(hex).ok())
            .collect::<Vec<_>>();
        Query::events()
            .kinds([fava::Kind::from_u16(0)])
            .context("profile-feed kinds")?
            .authors(pubkeys)
            .context("profile-feed authors")?
            .from_relays(self.profile_relays.iter().cloned())
            .context("profile-feed relay scope")
    }

    /// kinds 39000/39001/39002 for ALL groups, from the group relays.
    /// Used by the group-records watcher.
    pub(crate) fn all_group_records_query(&self) -> Result<Query> {
        Query::events()
            .kinds([
                fava::Kind::from_u16(39_000),
                fava::Kind::from_u16(39_001),
                fava::Kind::from_u16(39_002),
            ])
            .context("all-group-records kinds")?
            .only_from_relays(self.relays.iter().cloned())
            .context("all-group-records relay scope")
    }

    /// kinds 39000/39001/39002 for one group id, from the group relays.
    pub(crate) fn one_group_records_query(&self, group: &str) -> Result<Query> {
        let d = SingleLetterTag::from_char('d').expect("d is a single-letter tag");
        Query::events()
            .kinds([
                fava::Kind::from_u16(39_000),
                fava::Kind::from_u16(39_001),
                fava::Kind::from_u16(39_002),
            ])
            .context("one-group-records kinds")?
            .tag_values(d, [group])
            .context("one-group-records d-tag")?
            .only_from_relays(self.relays.iter().cloned())
            .context("one-group-records relay scope")
    }
}

/// Build a `filter` used by fabric callers that compose their own filter before
/// handing it to `fetch_profiles`.
pub(crate) fn filter(
    kinds: &[u16],
    authors: &[String],
    tags: &[(char, String)],
) -> Result<FetchFilter> {
    let pubkeys = authors
        .iter()
        .filter_map(|hex| nostr::key::PublicKey::from_hex(hex).ok())
        .collect::<Vec<_>>();
    Ok(FetchFilter {
        kinds: kinds.to_vec(),
        authors: pubkeys,
        tags: tags.to_vec(),
    })
}

/// A structured filter for bounded reads, replacing `nmp::Filter`.
#[derive(Clone, Default)]
pub(crate) struct FetchFilter {
    pub(crate) kinds: Vec<u16>,
    pub(crate) authors: Vec<nostr::key::PublicKey>,
    pub(crate) tags: Vec<(char, String)>,
}

impl FetchFilter {
    pub(crate) fn into_query(
        self,
        relays: impl IntoIterator<Item = RelayUrl>,
    ) -> Result<Query> {
        let mut q = Query::events();
        if !self.kinds.is_empty() {
            q = q
                .kinds(self.kinds.iter().map(|&k| fava::Kind::from_u16(k)))
                .context("fetch filter kinds")?;
        }
        if !self.authors.is_empty() {
            q = q.authors(self.authors).context("fetch filter authors")?;
        }
        for (name, value) in &self.tags {
            let key = SingleLetterTag::from_char(*name)
                .with_context(|| format!("invalid tag name {name:?} in fetch filter"))?;
            q = q.tag_values(key, [value.as_str()]).context("fetch filter tag")?;
        }
        q.from_relays(relays).context("fetch filter relay scope")
    }
}

#[cfg(test)]
#[path = "query/tests.rs"]
mod tests;
