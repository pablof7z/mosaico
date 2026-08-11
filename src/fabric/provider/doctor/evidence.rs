use nmp::{
    AccessContext, AcquisitionEvidence, ReceiptResult, RelayState, SourceEvidence, WriteOutcome,
};
use serde::Serialize;

use crate::nmp_host::read::{BoundedRead, BoundedReadTermination};

use super::ProbeStatus;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProbeStep {
    pub(crate) status: ProbeStatus,
    pub(crate) summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) terminal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) relays: Vec<RelayProbe>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) acquisition: Option<ReadProbe>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct RelayProbe {
    relay: String,
    state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReadProbe {
    termination: BoundedReadTermination,
    branches: Vec<ReadBranchProbe>,
}

#[derive(Debug, Clone, Serialize)]
struct ReadBranchProbe {
    sources: Vec<ReadSourceProbe>,
    shortfalls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ReadSourceProbe {
    relay: String,
    access: String,
    status: String,
    reconciled_through: Option<u64>,
}

pub(super) fn failed(summary: impl Into<String>) -> ProbeStep {
    ProbeStep {
        status: ProbeStatus::Failed,
        summary: summary.into(),
        terminal: None,
        relays: Vec::new(),
        acquisition: None,
    }
}

pub(super) fn skipped(summary: impl Into<String>) -> ProbeStep {
    ProbeStep {
        status: ProbeStatus::Skipped,
        summary: summary.into(),
        terminal: None,
        relays: Vec::new(),
        acquisition: None,
    }
}

pub(super) fn publish(event_id: &nostr::EventId, result: ReceiptResult) -> ProbeStep {
    let relays = result
        .relays
        .iter()
        .map(|(relay, state)| relay_probe(relay, state))
        .collect::<Vec<_>>();
    let verified = result.outcome == WriteOutcome::Settled
        && !result.relays.is_empty()
        && result
            .relays
            .values()
            .all(|state| matches!(state, RelayState::Published));
    let summary = if verified {
        format!(
            "{} relay(s) published {}",
            relays.len(),
            crate::util::pubkey_short(&event_id.to_hex())
        )
    } else {
        format!(
            "terminal {:?}; {}",
            result.outcome,
            relay_failure_summary(&result)
        )
    };
    ProbeStep {
        status: if verified {
            ProbeStatus::Verified
        } else {
            ProbeStatus::Failed
        },
        summary,
        terminal: Some(format!("{:?}", result.outcome)),
        relays,
        acquisition: None,
    }
}

pub(super) fn readback(
    read: BoundedRead,
    require_row: bool,
    description: impl std::fmt::Display,
) -> ProbeStep {
    let verified = read.termination == BoundedReadTermination::RelaySettled
        && (!require_row || !read.rows.is_empty());
    let summary = if verified {
        format!(
            "relay-settled {description} returned {} event(s)",
            read.rows.len()
        )
    } else if require_row
        && read.rows.is_empty()
        && read.termination == BoundedReadTermination::RelaySettled
    {
        format!("relay-settled marker was absent: 0 event(s) for {description}")
    } else {
        format!(
            "{description} ended as {:?} with {} cached/current event(s)",
            read.termination,
            read.rows.len()
        )
    };
    ProbeStep {
        status: if verified {
            ProbeStatus::Verified
        } else {
            ProbeStatus::Failed
        },
        summary,
        terminal: None,
        relays: Vec::new(),
        acquisition: Some(ReadProbe {
            termination: read.termination,
            branches: read.evidence.iter().map(read_branch_probe).collect(),
        }),
    }
}

fn relay_probe(relay: &nostr::RelayUrl, state: &RelayState) -> RelayProbe {
    let (label, reason) = match state {
        RelayState::Published => ("published", None),
        RelayState::Rejected { reason } => ("rejected", Some(reason.clone())),
        RelayState::AuthFailed { reason, .. } => ("auth_failed", Some(reason.clone())),
        RelayState::GaveUp => ("gave_up", None),
        RelayState::Waiting(waiting) => ("waiting", Some(format!("{waiting:?}"))),
        RelayState::Sent { .. } => ("sent", None),
    };
    RelayProbe {
        relay: relay.to_string(),
        state: label.into(),
        reason,
    }
}

fn relay_failure_summary(result: &ReceiptResult) -> String {
    if result.relays.is_empty() {
        return "no relay terminal result".into();
    }
    result
        .relays
        .iter()
        .filter_map(|(relay, state)| match state {
            RelayState::Published => None,
            RelayState::Rejected { reason } => Some(format!("{relay} rejected: {reason}")),
            RelayState::AuthFailed { reason, .. } => {
                Some(format!("{relay} authentication failed: {reason}"))
            }
            RelayState::GaveUp => Some(format!("{relay} gave up")),
            RelayState::Waiting(waiting) => Some(format!("{relay} still waiting: {waiting:?}")),
            RelayState::Sent { .. } => Some(format!("{relay} ended sent without an ACK")),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn read_branch_probe(evidence: &AcquisitionEvidence) -> ReadBranchProbe {
    ReadBranchProbe {
        sources: evidence.sources.iter().map(read_source_probe).collect(),
        shortfalls: evidence
            .shortfall
            .iter()
            .map(|shortfall| format!("{shortfall:?}"))
            .collect(),
    }
}

fn read_source_probe(source: &SourceEvidence) -> ReadSourceProbe {
    ReadSourceProbe {
        relay: source.relay.to_string(),
        access: match &source.access {
            AccessContext::Public => "public".into(),
            AccessContext::Nip42(pubkey) => format!("nip42:{pubkey}"),
        },
        status: format!("{:?}", source.status),
        reconciled_through: source
            .reconciled_through
            .map(|timestamp| timestamp.as_secs()),
    }
}

#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
