//! How a Mosaico question becomes an NMP live query.
//!
//! The whole point of this module is that the two host-scoping axes are
//! decided HERE, once, and never inherited by accident. `ReadRouting`
//! scopes which relays are ASKED; `CacheMode` scopes which locally cached
//! rows may ANSWER. Their defaults disagree — a `Demand` is `Explicit` only
//! when you say so, but it is `Agnostic` unless you say otherwise — and
//! mosaico#741 is exactly what happens when only the first one is set.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nmp::{
    Binding, CacheMode, Demand, IndexedTagName, LiveQuery, PublicKey, ReadRouting, RelayUrl,
};

use crate::reconcile::SubscriptionQuery;

use super::NmpHost;

impl NmpHost {
    /// The relays a NIP-29 group lives on, as NMP's own scope value. Every
    /// NIP-29 read is minted from here so the host set is named once and the
    /// per-host branch structure is NMP's to build, not Mosaico's to imitate.
    pub(crate) fn nip29_scope(&self) -> Result<nmp_nip29::RelayScope> {
        nmp_nip29::on(self.relays.iter().cloned())
            .map_err(|error| anyhow::anyhow!("NIP-29 relay scope: {error}"))
    }

    /// Watch the relay-signed records of every group matching `predicate`.
    ///
    /// The returned [`GroupObservation`](nmp_nip29::GroupObservation) is the
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
        predicate: nmp_nip29::GroupPredicate,
    ) -> Result<nmp_nip29::GroupObservation> {
        self.nip29_scope()?
            .observe(&self.engine, predicate, nmp_nip29::GroupRecord::ALL, None)
            .map_err(|error| anyhow::anyhow!("NIP-29 group records observation: {error}"))
    }

    /// Watch one group's relay-signed records through NMP's group facade.
    ///
    /// The returned observation is the state owner. Callers read its complete
    /// [`nmp_nip29::GroupSnapshot`] deliveries directly; Mosaico does not
    /// materialize or merge another group record set.
    pub(crate) fn observe_one_group_records(
        &self,
        group: &str,
    ) -> Result<nmp_nip29::GroupObservation> {
        self.nip29_scope()?
            .group(group)
            .observe(&self.engine, nmp_nip29::GroupRecord::ALL)
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
            .map(|host| nmp_nip29::group_records_at(host, &records, Some(predicate.clone()), None))
            .collect::<Vec<_>>();
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
        authenticate_as: Option<PublicKey>,
        cache: CacheMode,
    ) -> Result<LiveQuery> {
        let demand = if relays.is_empty() {
            let mut demand = Demand::new(filter, ReadRouting::Auto)?;
            demand.authenticate_as = authenticate_as;
            demand
        } else {
            let mut demand = Demand::new(
                filter,
                ReadRouting::Explicit(relays.iter().cloned().collect()),
            )?;
            demand.authenticate_as = authenticate_as;
            demand.cache = cache;
            demand
        };
        Ok(LiveQuery::single(demand))
    }

    /// kind:0 for the current member set, pinned to the profile relays,
    /// `Agnostic`. This is the retained profile feed's query (mosaico#837):
    /// one observation scoped to `authors:[members]`, draining into a small
    /// bounded per-pubkey profile map. kind:0 is self-authenticating, so the
    /// cache stays provenance-agnostic exactly like the single-profile read.
    pub(crate) fn profile_feed_query(&self, authors: &BTreeSet<String>) -> Result<LiveQuery> {
        let mut filter = kinds_filter(&BTreeSet::from([0u16]));
        filter.authors = Some(Binding::Literal(authors.clone()));
        self.host_pinned_query(&self.profile_relays, filter, None, CacheMode::Agnostic)
    }

    /// Open a caller-owned NMP observation over an already-built live query.
    /// The caller owns the [`nmp::Subscription`] and its drain. Used by
    /// retained feeds that keep their own projection rather than the daemon
    /// demux transition stream.
    pub(crate) fn observe_query(&self, query: LiveQuery) -> Result<nmp::Subscription> {
        self.engine
            .observe(query, None)
            .context("opening NMP observation")
    }

    pub(super) fn live_query(
        &self,
        query: &SubscriptionQuery,
        authenticate_as: Option<PublicKey>,
    ) -> Result<LiveQuery> {
        match query {
            SubscriptionQuery::GroupContentsMulti { groups, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter
                    .tags
                    .insert(indexed_tag('h')?, Binding::Literal(groups.clone()));
                self.host_pinned_query(&self.relays, filter, authenticate_as, CacheMode::Strict)
            }
            SubscriptionQuery::References { event_id, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter.tags.insert(
                    indexed_tag('e')?,
                    Binding::Literal(BTreeSet::from([event_id.clone()])),
                );
                self.host_pinned_query(&self.relays, filter, authenticate_as, CacheMode::Strict)
            }
            SubscriptionQuery::Profile { pubkey } => {
                let mut filter = kinds_filter(&BTreeSet::from([0u16]));
                filter.authors = Some(Binding::Literal(BTreeSet::from([pubkey.clone()])));
                self.host_pinned_query(
                    &self.profile_relays,
                    filter,
                    authenticate_as,
                    CacheMode::Agnostic,
                )
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
/// NIP-29 read door folds them (`nmp_nip29::read::one_live_query`).
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
