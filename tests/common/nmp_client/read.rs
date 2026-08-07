use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use nmp::{AccessContext, Binding, Demand, IndexedTagName, LiveQuery, RelayUrl, SourceAuthority};
use nostr::{Event, Filter};

/// Per-branch acquisition evidence is positionally indexed against
/// `LiveQuery::branches()` (#1108). One branch's shortfall is never masked by
/// a sibling's proof, so every branch must independently satisfy the test.
///
/// This is the same rule the daemon's own reads apply
/// (`src/nmp_host/read.rs`), expressed separately because this client stands
/// in for a third-party Nostr client and must not borrow the daemon's read
/// policy to prove the daemon works. Same question, same answer.
fn all_branches(
    evidence: &[nmp::AcquisitionEvidence],
    per_branch: impl Fn(&nmp::AcquisitionEvidence) -> bool,
) -> bool {
    !evidence.is_empty()
        && evidence.iter().all(|branch| {
            branch.shortfall.is_empty() && !branch.sources.is_empty() && per_branch(branch)
        })
}

/// A bounded read is over when every source has told us so — see
/// `src/nmp_host/read.rs` for why this is a disjunction of two independent
/// facts rather than a choice between them.
///
/// This file used to carry a LOOSER rule than its production twin: it accepted
/// a mixed set where some sources had proven a watermark and others were
/// merely `Requesting`, which production rejected. The divergence was real in
/// the text and inert in practice — `receive_window`'s only caller,
/// `NmpRelayClient::fetch_events`, strips the caller's NIP-01 `limit` exactly
/// as the daemon does, so its requests are coverage-eligible here too, and it
/// pins a single relay, so "some sources" and "all sources" name the same one
/// source. Neither copy needs its own rule now, and neither has one.
fn read_complete(evidence: &[nmp::AcquisitionEvidence]) -> bool {
    acquisition_settled(evidence) || acquisition_ready(evidence)
}

fn acquisition_settled(evidence: &[nmp::AcquisitionEvidence]) -> bool {
    all_branches(evidence, |branch| {
        branch
            .sources
            .iter()
            .all(|source| matches!(source.status, nmp::SourceStatus::FinishedStoredEvents))
    })
}

fn acquisition_ready(evidence: &[nmp::AcquisitionEvidence]) -> bool {
    all_branches(evidence, |branch| {
        branch
            .sources
            .iter()
            .all(|source| source.reconciled_through.is_some())
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
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match subscription.recv_timeout(remaining) {
            Ok(frame) => {
                let done = read_complete(&frame.evidence);
                latest = Some(frame);
                if done {
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
    // An empty row set is evidence of nothing on its own; it becomes an answer
    // only once the sources reported one. A read that merely ran out of time
    // reports a failure rather than an authoritative empty.
    if window.rows.is_empty() && !read_complete(&frame.evidence) {
        anyhow::bail!(
            "NMP test read ended without relay acquisition evidence: load={:?} evidence={:?}",
            window.load,
            frame.evidence
        );
    }
    Ok(window.rows.into_iter().map(|row| row.event).collect())
}
