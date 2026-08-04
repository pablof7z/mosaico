//! How a Mosaico question becomes an NMP live query.
//!
//! The whole point of this module is that the two host-scoping axes are
//! decided HERE, once, and never inherited by accident. `SourceAuthority`
//! scopes which relays are ASKED; `CacheMode` scopes which locally cached
//! rows may ANSWER. Their defaults disagree — a `Demand` is `Pinned` only
//! when you say so, but it is `Agnostic` unless you say otherwise — and
//! mosaico#741 is exactly what happens when only the first one is set.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nmp::{
    AccessContext, Binding, CacheMode, Demand, IndexedTagName, LiveQuery, RelayUrl, SourceAuthority,
};

use crate::reconcile::SubscriptionQuery;

use super::NmpHost;

impl NmpHost {
    /// The relays a NIP-29 group lives on, as NMP's own scope value. Every
    /// NIP-29 read is minted from here so the host set is named once and the
    /// per-host branch structure is NMP's to build, not Mosaico's to imitate.
    pub(crate) fn nip29_scope(&self) -> Result<nmp::nip29::RelayScope> {
        nmp::nip29::on(self.relays.iter().cloned())
            .map_err(|error| anyhow::anyhow!("NIP-29 relay scope: {error}"))
    }

    /// One group's read declaration for an app-supplied selection: one
    /// complete branch per host, each `Pinned` to that host alone and
    /// `CacheMode::Strict`, scoped by `#h`. NMP refuses a selection that
    /// already constrains `#h`, which is why the group id is a parameter here
    /// and never a tag the caller writes.
    pub(crate) fn group_contents_query(
        &self,
        group: &str,
        selection: nmp::Filter,
    ) -> Result<LiveQuery> {
        self.nip29_scope()?
            .group(group)
            .read(selection)
            .map_err(|error| anyhow::anyhow!("NIP-29 group read for {group:?}: {error}"))
    }

    /// Watch the relay-signed records of every group matching `predicate`.
    ///
    /// The returned [`GroupObservation`](nmp::nip29::GroupObservation) is the
    /// handle and Mosaico owns its lifetime: dropping it withdraws the demand.
    /// NMP deliberately caches nothing keyed by group on the app's behalf.
    ///
    /// Branches scale with HOSTS, not groups — every group this daemon watches
    /// rides the same one-branch-per-relay observation. The honest limit is at
    /// the wire, not here: the id leaf lowers to a `#d` set, and a relay may
    /// refuse or truncate a filter carrying very many values, so a daemon
    /// watching very many groups at once would need to shard across several
    /// observations.
    pub(crate) fn observe_group_records(
        &self,
        predicate: nmp::nip29::GroupPredicate,
    ) -> Result<nmp::nip29::GroupObservation> {
        self.nip29_scope()?
            .observe(&self.engine, predicate, nmp::nip29::GroupRecord::ALL)
            .map_err(|error| anyhow::anyhow!("NIP-29 group records observation: {error}"))
    }

    /// The relay-signed records describing ONE group — kinds 39000/39001/39002
    /// joined on `d`. One complete branch per host, `Pinned` and `Strict` at
    /// every nesting level, because these three kinds are signed by the RELAY
    /// and a row relay B served is no evidence about relay A's group.
    pub(crate) fn group_records_query(&self, group: &str) -> Result<LiveQuery> {
        let records = BTreeSet::from(nmp_nip29::GroupRecord::ALL);
        let predicate = Binding::Literal(BTreeSet::from([group.to_string()]));
        let branches = self
            .relays
            .iter()
            .map(|host| nmp_nip29::group_records_at(host, &records, predicate.clone()))
            .collect::<Vec<_>>();
        union_branches(branches)
    }

    /// Every group these hosts describe (kind:39000, unkeyed).
    ///
    /// NMP has no unpredicated group-listing constructor — `groups_where_at`
    /// requires a `d` predicate — so the branch is assembled here from NMP's
    /// own vocabulary rather than borrowed. It still stamps both axes per
    /// host, which is the property that matters. See the report accompanying
    /// mosaico#741: an `all_groups_at(host)` door would remove this.
    pub(crate) fn all_group_metadata_query(&self) -> Result<LiveQuery> {
        let selection = nmp::Filter {
            kinds: Some(BTreeSet::from([nmp_nip29::GROUP_METADATA_KIND])),
            ..nmp::Filter::default()
        };
        let branches = self
            .relays
            .iter()
            .map(|host| {
                let mut demand = Demand::new(
                    selection.clone(),
                    SourceAuthority::Pinned(BTreeSet::from([host.clone()])),
                    AccessContext::Public,
                )?;
                demand.cache = CacheMode::Strict;
                Ok(demand)
            })
            .collect::<Result<Vec<_>, nmp::DemandError>>()
            .map_err(|error| anyhow::anyhow!("group metadata listing: {error}"))?;
        union_branches(branches)
    }

    /// A read NMP's NIP-29 vocabulary does not mint, pinned to `relays` with
    /// an EXPLICIT cache mode.
    ///
    /// `cache` is a parameter and never a default because the default is
    /// `Agnostic` — "serve every matching cached row regardless of
    /// provenance" — and inheriting it silently is precisely the defect
    /// mosaico#741 records. The two rules Mosaico applies:
    ///
    /// * Pinned to the GROUP hosts → `Strict`. Those hosts are asked because
    ///   they are the authority for the answer; a row a different relay
    ///   served is not evidence about them. Mosaico's own not-yet-carried
    ///   writes stay visible regardless — NMP decides that by ours-versus-
    ///   foreign, not by carried-versus-uncarried.
    /// * Pinned to the PROFILE hosts → `Agnostic`. kind:0 is
    ///   self-authenticating, the answer does not depend on who served it,
    ///   and the indexer is in that set precisely so it can answer for
    ///   relays outside the app's own.
    pub(super) fn host_pinned_query(
        &self,
        relays: &BTreeSet<RelayUrl>,
        filter: nmp::Filter,
        access: AccessContext,
        cache: CacheMode,
    ) -> Result<LiveQuery> {
        let demand = if relays.is_empty() {
            Demand::from_filter(filter)
        } else {
            let mut demand = Demand::new(filter, SourceAuthority::Pinned(relays.clone()), access)?;
            demand.cache = cache;
            demand
        };
        Ok(LiveQuery::single(demand))
    }

    pub(super) fn live_query(
        &self,
        query: &SubscriptionQuery,
        access: AccessContext,
    ) -> Result<LiveQuery> {
        match query {
            SubscriptionQuery::AllGroupMetadata => self.all_group_metadata_query(),
            SubscriptionQuery::GroupContents { group, kinds } => {
                self.group_contents_query(group, kinds_filter(kinds))
            }
            SubscriptionQuery::Kinds { kinds } => {
                self.host_pinned_query(&self.relays, kinds_filter(kinds), access, CacheMode::Strict)
            }
            SubscriptionQuery::Mentions { pubkey, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter.tags.insert(
                    indexed_tag('p')?,
                    Binding::Literal(BTreeSet::from([pubkey.clone()])),
                );
                self.host_pinned_query(&self.relays, filter, access, CacheMode::Strict)
            }
            SubscriptionQuery::References { event_id, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter.tags.insert(
                    indexed_tag('e')?,
                    Binding::Literal(BTreeSet::from([event_id.clone()])),
                );
                self.host_pinned_query(&self.relays, filter, access, CacheMode::Strict)
            }
            SubscriptionQuery::Profile { pubkey } => {
                let mut filter = kinds_filter(&BTreeSet::from([0u16]));
                filter.authors = Some(Binding::Literal(BTreeSet::from([pubkey.clone()])));
                self.host_pinned_query(&self.profile_relays, filter, access, CacheMode::Agnostic)
            }
        }
    }
}

pub(super) fn kinds_filter(kinds: &BTreeSet<u16>) -> nmp::Filter {
    nmp::Filter {
        kinds: (!kinds.is_empty()).then(|| kinds.clone()),
        ..nmp::Filter::default()
    }
}

fn indexed_tag(name: char) -> Result<IndexedTagName> {
    IndexedTagName::new(name).with_context(|| format!("invalid indexed tag name {name}"))
}

/// One live query out of one complete branch per host, exactly as NMP's own
/// NIP-29 read door folds them (`nmp::nip29::read::one_live_query`).
fn union_branches(branches: Vec<Demand>) -> Result<LiveQuery> {
    let mut branches = branches;
    match branches.len() {
        0 => anyhow::bail!("no configured group host to read from"),
        1 => Ok(LiveQuery::single(
            branches.pop().expect("exactly one branch"),
        )),
        _ => LiveQuery::union(branches.into_iter().map(LiveQuery::single), None)
            .map_err(|error| anyhow::anyhow!("composing per-host read branches: {error}")),
    }
}

#[cfg(test)]
#[path = "query/tests.rs"]
mod tests;
