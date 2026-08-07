//! Bounded read projections through the daemon's sole NMP engine.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nmp::{AccessContext, AcquisitionEvidence, Binding, IndexedTagName, SourceStatus, Window};
use nostr::Event;

use super::NmpHost;

impl NmpHost {
    /// Read the relay-signed records describing ONE group — kinds
    /// 39000/39001/39002 keyed on `d`. Minted by NMP's NIP-29 discovery
    /// vocabulary, so every branch is pinned to one host AND strict about
    /// which cached rows may answer for it.
    pub(crate) async fn fetch_group_records(
        &self,
        group: &str,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        self.fetch_query(self.group_records_query(group)?, max_rows, timeout)
            .await
    }

    /// Read the relay-signed metadata for EVERY group these hosts describe.
    pub(crate) async fn fetch_all_group_metadata(
        &self,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        self.fetch_query(self.all_group_metadata_query()?, max_rows, timeout)
            .await
    }

    /// Read an app-chosen selection from INSIDE one group (`#h`-scoped).
    pub(crate) async fn fetch_in_group(
        &self,
        group: &str,
        filter: nmp::Filter,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        self.fetch_query(
            self.group_contents_query(group, strip_limit(filter))?,
            max_rows,
            timeout,
        )
        .await
    }

    /// Read bounded profile state from the configured app and indexer hosts.
    ///
    /// Deliberately provenance-agnostic: kind:0 is self-authenticating, the
    /// answer does not depend on which relay served it, and the indexer is
    /// pinned precisely so it can answer for relays outside the app's set.
    pub(crate) async fn fetch_profiles(
        &self,
        filter: nmp::Filter,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        let query = self.host_pinned_query(
            &self.profile_relays,
            strip_limit(filter),
            AccessContext::Public,
            nmp::CacheMode::Agnostic,
        )?;
        self.fetch_query(query, max_rows, timeout).await
    }

    async fn fetch_query(
        &self,
        query: nmp::LiveQuery,
        max_rows: usize,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        #[cfg(test)]
        if let Some(result) = self.test_io.take_read() {
            return result;
        }
        let bound = NonZeroUsize::new(max_rows).context("NMP read bound must be non-zero")?;
        let subscription = self
            .engine
            .observe(
                query,
                Some(Window::Expandable {
                    initial: bound,
                    max: bound,
                }),
            )
            .context("opening bounded NMP read")?;
        tokio::task::spawn_blocking(move || receive_bounded(subscription, timeout))
            .await
            .context("joining bounded NMP read")?
    }
}

/// The window owns the result bound. NMP rejects a competing NIP-01 limit.
fn strip_limit(mut filter: nmp::Filter) -> nmp::Filter {
    filter.limit = None;
    filter
}

/// `timeout` is the caller's FAILURE bound and nothing else: it caps how long
/// a read may hang, and a read that hits it yields a snapshot only if the
/// evidence independently justifies one. It is never the thing that decides a
/// read is finished — that is `acquisition_settled`, a fact the relays report.
fn receive_bounded(subscription: nmp::Subscription, timeout: Duration) -> Result<Vec<Event>> {
    let deadline = Instant::now() + timeout;
    let mut latest = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return finish_latest(latest);
        }
        match subscription.recv_timeout(remaining) {
            Ok(frame) => {
                let done = read_complete(&frame.evidence);
                latest = Some(frame);
                if done {
                    return finish_latest(latest);
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return finish_latest(latest),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return finish_latest(latest).context("NMP read disconnected")
            }
        }
    }
}

fn finish_latest(frame: Option<nmp::Frame>) -> Result<Vec<Event>> {
    let frame = frame.context("NMP read produced no snapshot")?;
    let window = frame.window.context("NMP bounded read had no window")?;
    // An empty row set is evidence of nothing on its own. It becomes an answer
    // only once the sources reported one — either they finished answering or
    // they proved the window. A read that merely ran out of time reports a
    // failure, so no clock can manufacture an authoritative empty.
    if window.rows.is_empty() && !read_complete(&frame.evidence) {
        anyhow::bail!(
            "NMP read ended without a usable relay acquisition attempt: {:?}",
            frame.evidence
        );
    }
    Ok(window.rows.into_iter().map(|row| row.event).collect())
}

/// Per-branch acquisition evidence is positionally indexed against
/// `LiveQuery::branches()` (#1108). One branch's shortfall is never masked by
/// a sibling's proof, so every test below is the conjunction over branches and
/// never a fold that loses which branch failed.
///
/// The same rule, verbatim, backs the e2e test client's reads
/// (`tests/common/nmp_client/read.rs`). The two are separate consumers of the
/// NMP facade on purpose — the test client stands in for a third-party Nostr
/// client and must not borrow the daemon's read policy — but the question they
/// ask is one question, and they answer it identically.
fn all_branches(
    evidence: &[AcquisitionEvidence],
    per_branch: impl Fn(&AcquisitionEvidence) -> bool,
) -> bool {
    !evidence.is_empty()
        && evidence.iter().all(|branch| {
            branch.shortfall.is_empty() && !branch.sources.is_empty() && per_branch(branch)
        })
}

/// A bounded read is over when every source has told us so. Two independent
/// facts each end it, and NMP keeps them independent in both directions
/// (nmp#1235), so this is a disjunction rather than a choice:
///
/// - `acquisition_settled` — every source reached NIP-01's end of stored
///   events. It sent everything it had for the question it was asked.
/// - `acquisition_ready` — every source proved a watermark across the query's
///   window. A read whose coverage is already proven need not wait for the
///   wire.
fn read_complete(evidence: &[AcquisitionEvidence]) -> bool {
    acquisition_settled(evidence) || acquisition_ready(evidence)
}

/// Every source finished answering — the fact Mosaico used to guess at with a
/// 500ms quiet period, because `SourceStatus` could not say it (nmp#1235).
/// Deliberately NOT a completeness verdict: a source that finishes having sent
/// nothing, and proved nothing, still finished, and that is precisely what
/// makes an empty snapshot usable.
fn acquisition_settled(evidence: &[AcquisitionEvidence]) -> bool {
    all_branches(evidence, |branch| {
        branch
            .sources
            .iter()
            .all(|source| matches!(source.status, SourceStatus::FinishedStoredEvents))
    })
}

fn acquisition_ready(evidence: &[AcquisitionEvidence]) -> bool {
    all_branches(evidence, |branch| {
        branch
            .sources
            .iter()
            .all(|source| source.reconciled_through.is_some())
    })
}

pub(crate) fn filter(
    kinds: &[u16],
    authors: &[String],
    tags: &[(char, String)],
) -> Result<nmp::Filter> {
    let mut indexed = BTreeMap::<IndexedTagName, BTreeSet<String>>::new();
    for (name, value) in tags {
        let name = IndexedTagName::new(*name)
            .with_context(|| format!("invalid indexed Nostr tag {name:?}"))?;
        indexed.entry(name).or_default().insert(value.clone());
    }
    Ok(nmp::Filter {
        kinds: (!kinds.is_empty()).then(|| kinds.iter().copied().collect()),
        authors: (!authors.is_empty()).then(|| Binding::Literal(authors.iter().cloned().collect())),
        tags: indexed
            .into_iter()
            .map(|(name, values)| (name, Binding::Literal(values)))
            .collect(),
        ..nmp::Filter::default()
    })
}

#[cfg(test)]
mod tests;
