//! Bounded read projections through the daemon's sole NMP engine.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nmp::{AccessContext, AcquisitionEvidence, Binding, IndexedTagName, SourceStatus, Window};
use nostr::Event;

use super::NmpHost;

const SNAPSHOT_QUIET_PERIOD: Duration = Duration::from_millis(500);

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

fn receive_bounded(subscription: nmp::Subscription, timeout: Duration) -> Result<Vec<Event>> {
    let deadline = Instant::now() + timeout;
    let mut latest = None;
    let mut quiet_deadline: Option<Instant> = None;
    loop {
        let now = Instant::now();
        let next_deadline = quiet_deadline
            .map(|quiet| quiet.min(deadline))
            .unwrap_or(deadline);
        let remaining = next_deadline.saturating_duration_since(now);
        if remaining.is_zero() {
            return finish_latest(latest);
        }
        match subscription.recv_timeout(remaining) {
            Ok(frame) => {
                let ready = acquisition_ready(&frame.evidence);
                quiet_deadline = acquisition_active(&frame.evidence)
                    .then(|| Instant::now() + SNAPSHOT_QUIET_PERIOD);
                latest = Some(frame);
                if ready {
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
    if window.rows.is_empty() && !empty_result_usable(&frame.evidence) {
        anyhow::bail!(
            "NMP read ended without a usable relay acquisition attempt: {:?}",
            frame.evidence
        );
    }
    Ok(window.rows.into_iter().map(|row| row.event).collect())
}

/// Every branch of the live query must be ready. Per-branch evidence is
/// positionally indexed against `LiveQuery::branches()` (#1108); one branch's
/// shortfall is never masked by a sibling's proof, so readiness is the
/// conjunction over branches and never a fold that loses which branch failed.
fn acquisition_ready(evidence: &[AcquisitionEvidence]) -> bool {
    !evidence.is_empty() && evidence.iter().all(branch_ready)
}

fn branch_ready(evidence: &AcquisitionEvidence) -> bool {
    evidence.shortfall.is_empty()
        && !evidence.sources.is_empty()
        && evidence
            .sources
            .iter()
            .all(|source| source.reconciled_through.is_some())
}

fn empty_result_usable(evidence: &[AcquisitionEvidence]) -> bool {
    acquisition_ready(evidence)
        // NMP intentionally exposes source facts rather than a global
        // completeness verdict. After an event-driven quiet period, an active
        // request with no routing shortfall is Mosaico's policy for accepting a
        // bounded snapshot; connection/AUTH failures still wait and fail.
        || acquisition_active(evidence)
}

fn acquisition_active(evidence: &[AcquisitionEvidence]) -> bool {
    !evidence.is_empty() && evidence.iter().all(branch_active)
}

fn branch_active(evidence: &AcquisitionEvidence) -> bool {
    evidence.shortfall.is_empty()
        && !evidence.sources.is_empty()
        && evidence
            .sources
            .iter()
            .all(|source| matches!(source.status, SourceStatus::Requesting))
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
mod tests {
    use super::*;

    #[test]
    fn filter_preserves_multiple_indexed_constraints() {
        let filter = filter(
            &[1],
            &["ab".repeat(32)],
            &[('h', "group".into()), ('t', "marker".into())],
        )
        .unwrap();
        assert_eq!(filter.kinds, Some(BTreeSet::from([1])));
        assert!(filter.authors.is_some());
        assert_eq!(filter.tags.len(), 2);
    }
}
