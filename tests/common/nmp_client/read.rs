use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use nmp::{AccessContext, Binding, Demand, IndexedTagName, LiveQuery, RelayUrl, SourceAuthority};
use nostr::{Event, Filter};

const SNAPSHOT_QUIET_PERIOD: Duration = Duration::from_millis(500);

/// Per-branch acquisition evidence is positionally indexed against
/// `LiveQuery::branches()` (#1108). One branch's shortfall is never masked by
/// a sibling's proof, so every branch must independently satisfy the test.
fn all_branches(
    evidence: &[nmp::AcquisitionEvidence],
    per_branch: impl Fn(&nmp::AcquisitionEvidence) -> bool,
) -> bool {
    !evidence.is_empty()
        && evidence.iter().all(|branch| {
            branch.shortfall.is_empty() && !branch.sources.is_empty() && per_branch(branch)
        })
}

pub(super) fn pinned_query(
    relay: RelayUrl,
    filter: nmp::Filter,
    access: AccessContext,
) -> Result<LiveQuery> {
    Ok(LiveQuery::single(Demand::new(
        filter,
        SourceAuthority::Pinned(BTreeSet::from([relay])),
        access,
    )?))
}

pub(super) fn nmp_filter(filter: Filter) -> Result<nmp::Filter> {
    if filter.search.is_some() {
        anyhow::bail!("NMP test client does not support NIP-50 search filters");
    }
    let tags = filter
        .generic_tags
        .into_iter()
        .map(|(name, values)| {
            let name = IndexedTagName::new(name.as_char())
                .context("nostr filter contained an invalid indexed tag")?;
            Ok((name, Binding::Literal(values)))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(nmp::Filter {
        ids: filter
            .ids
            .map(|ids| Binding::Literal(ids.into_iter().map(|id| id.to_hex()).collect())),
        authors: filter
            .authors
            .map(|xs| Binding::Literal(xs.into_iter().map(|x| x.to_hex()).collect())),
        kinds: filter
            .kinds
            .map(|kinds| kinds.into_iter().map(|kind| kind.as_u16()).collect()),
        tags,
        since: filter.since.map(|timestamp| timestamp.as_secs()),
        until: filter.until.map(|timestamp| timestamp.as_secs()),
        limit: filter.limit,
    })
}

pub(super) fn receive_window(
    subscription: nmp::Subscription,
    timeout: Duration,
) -> Result<Vec<Event>> {
    let deadline = Instant::now() + timeout;
    let mut latest = None;
    let mut quiet_deadline: Option<Instant> = None;
    loop {
        let next_deadline = quiet_deadline
            .map(|quiet| quiet.min(deadline))
            .unwrap_or(deadline);
        let remaining = next_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match subscription.recv_timeout(remaining) {
            Ok(frame) => {
                let acquisition_ready = all_branches(&frame.evidence, |evidence| {
                    evidence
                        .sources
                        .iter()
                        .all(|source| source.reconciled_through.is_some())
                });
                let acquisition_active = all_branches(&frame.evidence, |evidence| {
                    evidence
                        .sources
                        .iter()
                        .all(|source| matches!(source.status, nmp::SourceStatus::Requesting))
                });
                quiet_deadline = acquisition_active.then(|| Instant::now() + SNAPSHOT_QUIET_PERIOD);
                latest = Some(frame);
                if acquisition_ready {
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("NMP test read disconnected")
            }
        }
    }
    let frame = latest.context("NMP test read produced no window")?;
    let window = frame
        .window
        .context("NMP test bounded read had no window")?;
    let usable_empty = all_branches(&frame.evidence, |evidence| {
        evidence.sources.iter().all(|source| {
            source.reconciled_through.is_some()
                || matches!(source.status, nmp::SourceStatus::Requesting)
        })
    });
    if window.rows.is_empty() && !usable_empty {
        anyhow::bail!(
            "NMP test read ended without relay acquisition evidence: load={:?} evidence={:?}",
            window.load,
            frame.evidence
        );
    }
    Ok(window.rows.into_iter().map(|row| row.event).collect())
}
