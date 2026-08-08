use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use nmp::{
    AccessContext, Binding, CacheMode, Demand, Filter, Freshness, IndexedTagName, LiveQuery,
    SourceAuthority,
};
use nmp_grammar::{ConcreteFilter, ContextualAtom};
use nostr::{Alphabet, Keys, Kind, RelayUrl, SingleLetterTag};

use crate::args::{Args, Topology};

pub(crate) struct Workload {
    pub(crate) identities: Vec<Keys>,
    identity_hex: Vec<String>,
    groups: Vec<String>,
    relay: RelayUrl,
    retained: usize,
    mailboxes: usize,
    shard_size: usize,
}

impl Workload {
    pub(crate) fn new(args: &Args) -> Result<Self> {
        let identity_count = args.mailboxes.max(args.profile_burst).max(1);
        let identities = (0..identity_count)
            .map(|index| deterministic_keys(args.seed, index))
            .collect::<Result<Vec<_>>>()?;
        let identity_hex = identities
            .iter()
            .map(|keys| keys.public_key().to_hex())
            .collect();
        let groups = (0..args.retained.saturating_sub(args.mailboxes))
            .map(|index| format!("stress-group-{index:04}"))
            .collect();
        Ok(Self {
            identities,
            identity_hex,
            groups,
            relay: RelayUrl::parse("wss://nmp-stress.invalid")?,
            retained: args.retained,
            mailboxes: args.mailboxes,
            shard_size: args.shard_size,
        })
    }

    pub(crate) fn relay(&self) -> &RelayUrl {
        &self.relay
    }

    pub(crate) fn retained_queries(&self, topology: Topology) -> Result<Vec<LiveQuery>> {
        let mut queries = Vec::new();
        for values in self.partition(&self.identity_hex[..self.mailboxes], topology) {
            queries.push(self.tag_query('p', values)?);
        }
        for values in self.partition(&self.groups, topology) {
            queries.push(self.tag_query('h', values)?);
        }
        Ok(queries)
    }

    pub(crate) fn retained_demands(&self, topology: Topology) -> Result<Vec<Demand>> {
        self.retained_queries(topology).map(|queries| {
            queries
                .into_iter()
                .map(|query| query.branches()[0].clone())
                .collect()
        })
    }

    pub(crate) fn retained_live_queries(&self, topology: Topology) -> Result<Vec<LiveQuery>> {
        self.retained_demands(topology).map(|demands| {
            demands
                .into_iter()
                .map(|mut demand| {
                    demand.freshness = Freshness::Live;
                    LiveQuery::single(demand)
                })
                .collect()
        })
    }

    pub(crate) fn profile_query(&self, index: usize) -> Result<LiveQuery> {
        let author = self.identity_hex[index % self.identity_hex.len()].clone();
        let filter = Filter {
            kinds: Some(BTreeSet::from([0u16])),
            authors: Some(Binding::Literal(BTreeSet::from([author]))),
            ..Filter::default()
        };
        self.cache_only_query(filter, CacheMode::Agnostic)
    }

    pub(crate) fn router_atoms(&self, topology: Topology) -> Result<BTreeSet<ContextualAtom>> {
        let mut atoms = BTreeSet::new();
        for values in self.partition(&self.identity_hex[..self.mailboxes], topology) {
            atoms.insert(self.tag_atom('p', values)?);
        }
        for values in self.partition(&self.groups, topology) {
            atoms.insert(self.tag_atom('h', values)?);
        }
        Ok(atoms)
    }

    pub(crate) fn store_filters(&self, topology: Topology) -> Vec<nostr::Filter> {
        let mut filters = Vec::new();
        for values in self.partition(&self.identity_hex[..self.mailboxes], topology) {
            filters.push(store_tag_filter('p', values));
        }
        for values in self.partition(&self.groups, topology) {
            filters.push(store_tag_filter('h', values));
        }
        filters
    }

    pub(crate) fn profile_store_filter(&self, index: usize) -> nostr::Filter {
        nostr::Filter::new()
            .kind(Kind::Metadata)
            .author(self.identities[index % self.identities.len()].public_key())
    }

    pub(crate) fn semantic_values(&self) -> usize {
        self.retained
    }

    fn partition<'a>(&self, values: &'a [String], topology: Topology) -> Vec<&'a [String]> {
        match topology {
            Topology::PerIdentity => values.chunks(1).collect(),
            Topology::Sharded => values.chunks(self.shard_size).collect(),
        }
    }

    fn tag_query(&self, tag: char, values: &[String]) -> Result<LiveQuery> {
        let filter = Filter {
            kinds: Some(BTreeSet::from([9u16])),
            tags: BTreeMap::from([(
                IndexedTagName::new(tag).context("indexed tag")?,
                Binding::Literal(values.iter().cloned().collect()),
            )]),
            ..Filter::default()
        };
        self.cache_only_query(filter, CacheMode::Strict)
    }

    fn cache_only_query(&self, filter: Filter, cache: CacheMode) -> Result<LiveQuery> {
        let mut demand = Demand::new(
            filter,
            SourceAuthority::Pinned(BTreeSet::from([self.relay.clone()])),
            AccessContext::Public,
        )?;
        demand.cache = cache;
        demand.freshness = Freshness::CacheOnly;
        Ok(LiveQuery::single(demand))
    }

    fn tag_atom(&self, tag: char, values: &[String]) -> Result<ContextualAtom> {
        Ok(ContextualAtom {
            filter: ConcreteFilter {
                kinds: Some(BTreeSet::from([9u16])),
                tags: BTreeMap::from([(
                    IndexedTagName::new(tag).context("indexed tag")?,
                    values.iter().cloned().collect(),
                )]),
                ..ConcreteFilter::default()
            },
            source: SourceAuthority::Pinned(BTreeSet::from([self.relay.clone()])),
            access: AccessContext::Public,
            routing_evidence: BTreeSet::new(),
        })
    }
}

fn deterministic_keys(seed: u64, index: usize) -> Result<Keys> {
    let scalar = seed.wrapping_add(index as u64).wrapping_rem(u64::MAX - 1) + 1;
    Keys::parse(&format!("{scalar:064x}"))
        .map_err(anyhow::Error::from)
        .context("constructing deterministic fixture identity")
}

fn store_tag_filter(tag: char, values: &[String]) -> nostr::Filter {
    let tag = match tag {
        'p' => SingleLetterTag::lowercase(Alphabet::P),
        'h' => SingleLetterTag::lowercase(Alphabet::H),
        _ => unreachable!("workload only uses p and h tags"),
    };
    nostr::Filter::new()
        .kind(Kind::from(9u16))
        .custom_tags(tag, values.iter().cloned())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[test]
    fn sharding_preserves_values_and_reduces_handles() {
        let args = Args::parse_from([
            "stress",
            "--retained",
            "10",
            "--mailboxes",
            "8",
            "--profile-burst",
            "1",
            "--corpus-rows",
            "10",
            "--shard-size",
            "4",
        ]);
        let workload = Workload::new(&args).unwrap();
        assert_eq!(
            workload
                .retained_queries(Topology::PerIdentity)
                .unwrap()
                .len(),
            10
        );
        assert_eq!(
            workload.retained_queries(Topology::Sharded).unwrap().len(),
            3
        );
        assert_eq!(workload.semantic_values(), 10);
    }
}
