use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use nmp::{
    AccessContext, Binding, CacheMode, Demand, Derived, Filter, Freshness, IndexedTagName,
    LiveQuery, Selector, SourceAuthority,
};
use nmp_grammar::{ConcreteFilter, ContextualAtom};

use super::Workload;
use crate::args::DemandShape;

impl Workload {
    pub(crate) fn live_authors_query(
        &self,
        kinds: impl IntoIterator<Item = u16>,
        author_indexes: impl IntoIterator<Item = usize>,
    ) -> Result<LiveQuery> {
        let authors = author_indexes
            .into_iter()
            .map(|index| self.identity_hex[index % self.identity_hex.len()].clone())
            .collect();
        let filter = Filter {
            kinds: Some(kinds.into_iter().collect()),
            authors: Some(Binding::Literal(authors)),
            ..Filter::default()
        };
        self.query(filter, CacheMode::Agnostic, Freshness::Live)
    }

    pub(crate) fn concrete_authors_filter(
        &self,
        kinds: impl IntoIterator<Item = u16>,
        author_indexes: impl IntoIterator<Item = usize>,
    ) -> ConcreteFilter {
        ConcreteFilter {
            kinds: Some(kinds.into_iter().collect()),
            authors: Some(
                author_indexes
                    .into_iter()
                    .map(|index| self.identity_hex[index % self.identity_hex.len()].clone())
                    .collect(),
            ),
            ..ConcreteFilter::default()
        }
    }

    pub(crate) fn matrix_queries(
        &self,
        count: usize,
        shape: DemandShape,
    ) -> Result<Vec<LiveQuery>> {
        match shape {
            DemandShape::All => unreachable!("matrix expands all demand shapes before building"),
            DemandShape::ExactDuplicate => {
                let query = self.live_tag_query('p', &self.identity_hex[..1])?;
                Ok(vec![query; count])
            }
            DemandShape::CompatibleDistinct => (0..count)
                .map(|index| self.live_tag_query('p', &self.identity_hex[index..=index]))
                .collect(),
            DemandShape::ProfileAuthors => (0..count)
                .map(|index| self.live_profile_query(index))
                .collect(),
            DemandShape::LimitedIncompatible => (0..count)
                .map(|index| self.limited_incompatible_query(index))
                .collect(),
            DemandShape::UnlimitedMultiAxisIncompatible => (0..count)
                .map(|index| self.unlimited_multi_axis_query(index))
                .collect(),
        }
    }

    pub(crate) fn demand_key_distinct_queries(&self) -> Result<Vec<LiveQuery>> {
        let value = self.identity_hex[0].clone();
        let filter = |since, until, limit| Filter {
            kinds: Some(BTreeSet::from([9u16])),
            tags: BTreeMap::from([(
                IndexedTagName::new('p').expect("p is an indexed tag"),
                Binding::Literal(BTreeSet::from([value.clone()])),
            )]),
            since,
            until,
            limit,
            ..Filter::default()
        };
        Ok(vec![
            self.query(filter(None, None, None), CacheMode::Strict, Freshness::Live)?,
            self.query(
                filter(Some(1_700_000_000), Some(1_700_000_100), Some(1)),
                CacheMode::Strict,
                Freshness::Live,
            )?,
        ])
    }

    pub(crate) fn live_cache_pairs(&self, pairs: usize) -> Result<Vec<(LiveQuery, LiveQuery)>> {
        (0..pairs)
            .map(|index| {
                let values = &self.identity_hex[index..=index];
                Ok((
                    self.tag_query_with_freshness('p', values, Freshness::Live)?,
                    self.tag_query_with_freshness('p', values, Freshness::CacheOnly)?,
                ))
            })
            .collect()
    }

    pub(crate) fn nested_same_demand_query(
        &self,
        index: usize,
        outer_freshness: Freshness,
    ) -> Result<LiveQuery> {
        let selection = self.profile_filter(index);
        let mut inner = Demand::new(
            selection,
            SourceAuthority::Pinned(BTreeSet::from([self.relay.clone()])),
            AccessContext::Public,
        )?;
        inner.freshness = Freshness::Live;
        let outer_selection = Filter {
            kinds: Some(BTreeSet::from([0u16])),
            authors: Some(Binding::Derived(Box::new(Derived {
                inner,
                project: Selector::Authors,
            }))),
            ..Filter::default()
        };
        self.query(outer_selection, CacheMode::Agnostic, outer_freshness)
    }

    pub(crate) fn profile_atom(&self, index: usize) -> ContextualAtom {
        ContextualAtom {
            filter: ConcreteFilter {
                kinds: Some(BTreeSet::from([0u16])),
                authors: Some(BTreeSet::from([self.identity_hex
                    [index % self.identity_hex.len()]
                .clone()])),
                ..ConcreteFilter::default()
            },
            source: SourceAuthority::Pinned(BTreeSet::from([self.relay.clone()])),
            access: AccessContext::Public,
            routing_evidence: BTreeSet::new(),
        }
    }

    fn profile_filter(&self, index: usize) -> Filter {
        Filter {
            kinds: Some(BTreeSet::from([0u16])),
            authors: Some(Binding::Literal(BTreeSet::from([self.identity_hex
                [index % self.identity_hex.len()]
            .clone()]))),
            ..Filter::default()
        }
    }

    fn live_profile_query(&self, index: usize) -> Result<LiveQuery> {
        self.query(
            self.profile_filter(index),
            CacheMode::Agnostic,
            Freshness::Live,
        )
    }

    fn limited_incompatible_query(&self, index: usize) -> Result<LiveQuery> {
        let filter = Filter {
            kinds: Some(BTreeSet::from([9u16])),
            tags: BTreeMap::from([(
                IndexedTagName::new('p').context("indexed tag")?,
                Binding::Literal(BTreeSet::from([self.identity_hex[index].clone()])),
            )]),
            limit: Some(1),
            ..Filter::default()
        };
        self.query(filter, CacheMode::Strict, Freshness::Live)
    }

    fn unlimited_multi_axis_query(&self, index: usize) -> Result<LiveQuery> {
        let filter = Filter {
            kinds: Some(BTreeSet::from([9u16])),
            tags: BTreeMap::from([(
                IndexedTagName::new('p').context("indexed tag")?,
                Binding::Literal(BTreeSet::from([self.identity_hex[index].clone()])),
            )]),
            since: Some(1_600_000_000 + index as u64),
            ..Filter::default()
        };
        self.query(filter, CacheMode::Strict, Freshness::Live)
    }
}
