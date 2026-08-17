//! Provenance-bearing delivery from NMP observations to Mosaico's reducer.

use std::collections::BTreeSet;

use nmp::{AccessContext, ObservationCancel, RelayUrl, SourceStatus};
use serde::Serialize;

/// One NMP frame's row transition, carried and applied as a unit.
///
/// Replacements deliver `Removed(old)` and `Added(new)` in one frame. Delta
/// order is event-id order, not causal order, so the reducer always applies
/// removals before additions while retaining the frame's observation identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MaterializationPhase {
    Frame,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedRow {
    pub(crate) event: nostr::Event,
    pub(crate) sources: BTreeSet<RelayUrl>,
}

impl ObservedRow {
    pub(crate) fn sources_json(&self) -> anyhow::Result<String> {
        sources_json(&self.sources)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceGrowth {
    pub(crate) id: nostr::EventId,
    pub(crate) sources: BTreeSet<RelayUrl>,
}

impl SourceGrowth {
    pub(crate) fn sources_json(&self) -> anyhow::Result<String> {
        sources_json(&self.sources)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MaterializationBatch {
    pub(crate) observation_id: String,
    pub(crate) generation: u64,
    pub(crate) phase: MaterializationPhase,
    pub(crate) removed: Vec<nostr::EventId>,
    pub(crate) added: Vec<ObservedRow>,
    pub(crate) sources_grew: Vec<SourceGrowth>,
    pub(crate) evidence: Vec<nmp::AcquisitionEvidence>,
}

impl MaterializationBatch {
    pub(super) fn from_frame(observation_id: &str, generation: u64, frame: &nmp::Frame) -> Self {
        let mut batch = Self {
            observation_id: observation_id.to_string(),
            generation,
            phase: MaterializationPhase::Frame,
            removed: Vec::new(),
            added: Vec::new(),
            sources_grew: Vec::new(),
            evidence: frame.evidence.clone(),
        };
        for delta in &frame.deltas {
            match delta {
                nmp::RowDelta::Added(row) => batch.added.push(ObservedRow {
                    event: row.event.clone(),
                    sources: row.sources.clone(),
                }),
                nmp::RowDelta::Removed(id) => batch.removed.push(*id),
                nmp::RowDelta::SourcesGrew { id, sources } => {
                    batch.sources_grew.push(SourceGrowth {
                        id: *id,
                        sources: sources.clone(),
                    });
                }
            }
        }
        batch
    }

    pub(super) fn closed(observation_id: String, generation: u64) -> Self {
        Self {
            observation_id,
            generation,
            phase: MaterializationPhase::Closed,
            removed: Vec::new(),
            added: Vec::new(),
            sources_grew: Vec::new(),
            evidence: Vec::new(),
        }
    }

    pub(crate) fn evidence_json(&self) -> anyhow::Result<String> {
        scoped_evidence_json(&self.evidence)
    }

    pub(crate) fn relay_settled(&self) -> bool {
        relay_settled(&self.evidence)
    }
}

pub(crate) fn scoped_evidence_json(
    evidence: &[nmp::AcquisitionEvidence],
) -> anyhow::Result<String> {
    let branches = evidence
        .iter()
        .map(|branch| EvidenceBranch {
            sources: branch
                .sources
                .iter()
                .map(|source| EvidenceSource {
                    relay: source.relay.to_string(),
                    access: match &source.access {
                        AccessContext::Public => "public".into(),
                        AccessContext::Nip42(pubkey) => format!("nip42:{pubkey}"),
                    },
                    status: format!("{:?}", source.status),
                    reconciled_through: source
                        .reconciled_through
                        .map(|timestamp| timestamp.as_secs()),
                })
                .collect(),
            shortfalls: branch
                .shortfall
                .iter()
                .map(|shortfall| format!("{shortfall:?}"))
                .collect(),
        })
        .collect::<Vec<_>>();
    Ok(serde_json::to_string(&branches)?)
}

pub(crate) fn relay_settled(evidence: &[nmp::AcquisitionEvidence]) -> bool {
    !evidence.is_empty()
        && evidence.iter().all(|branch| {
            branch.shortfall.is_empty()
                && !branch.sources.is_empty()
                && branch
                    .sources
                    .iter()
                    .all(|source| matches!(source.status, SourceStatus::FinishedStoredEvents))
        })
}

pub(super) struct ActiveObservation {
    pub(super) generation: u64,
    pub(super) cancel: ObservationCancel,
}

fn sources_json(sources: &BTreeSet<RelayUrl>) -> anyhow::Result<String> {
    Ok(serde_json::to_string(
        &sources.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )?)
}

#[derive(Serialize)]
struct EvidenceBranch {
    sources: Vec<EvidenceSource>,
    shortfalls: Vec<String>,
}

#[derive(Serialize)]
struct EvidenceSource {
    relay: String,
    access: String,
    status: String,
    reconciled_through: Option<u64>,
}
