//! Reading Mosaico's own outstanding writes back out of NMP.
//!
//! The background receipt observer watches a stream for a bounded window and
//! its evidence dies with the process. That was tolerable while the foreground
//! path blocked and reported failures to the caller; it is not, now that every
//! write leaves optimistically. NMP's publish queue is the durable half — it
//! survives restart, and it is the only place a write parked on a missing
//! signer, or permanently refused at acceptance, is still visible an hour
//! later.
//!
//! This READS and does not remove. Removal is a termination path for an
//! obligation a person asked for, and a diagnostic is the wrong place to
//! decide that; `mosaico doctor` names the entries and leaves the call to a
//! human.

use nmp::{PublishQueueEntry, SigningState, WriteOutcome};
use serde::Serialize;

use super::super::NmpHost;

/// How many stuck writes are named before the list is truncated. The queue is
/// unbounded (NMP #46) and a doctor report is read by a person.
const NAMED_STUCK_WRITES: usize = 8;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct PublishQueueSnapshot {
    /// Every entry NMP still holds, terminal or not.
    pub(crate) entries: usize,
    /// Entries with no outcome yet: NMP is still working on them.
    pub(crate) outstanding: usize,
    /// Entries nothing is going to move without someone acting.
    pub(crate) stuck: Vec<StuckWrite>,
    /// How many were stuck in total, when `stuck` was truncated.
    pub(crate) stuck_total: usize,
    /// The queue could not be read. Distinct from an empty queue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) unreadable: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct StuckWrite {
    pub(crate) event_id: String,
    pub(crate) accepted_at: u64,
    /// NMP's own account of why, never a Mosaico paraphrase.
    pub(crate) reason: String,
}

impl NmpHost {
    /// Summarize what this daemon still owes, from NMP's durable queue.
    ///
    /// Deliberately not on any hot path: it enumerates every retained receipt,
    /// and retained receipts grow without bound (NMP #46).
    pub(crate) fn publish_queue_snapshot(&self) -> PublishQueueSnapshot {
        match self.engine.publish_queue() {
            Ok(entries) => summarize(&entries),
            Err(error) => PublishQueueSnapshot {
                entries: 0,
                outstanding: 0,
                stuck: Vec::new(),
                stuck_total: 0,
                unreadable: Some(error.to_string()),
            },
        }
    }
}

pub(super) fn summarize(entries: &[PublishQueueEntry]) -> PublishQueueSnapshot {
    let mut stuck = Vec::new();
    let mut outstanding = 0;
    for entry in entries {
        if !entry.is_terminal() {
            outstanding += 1;
        }
        if let Some(reason) = stuck_reason(entry) {
            stuck.push(StuckWrite {
                event_id: entry.event_id.to_hex(),
                accepted_at: entry.accepted_at.as_secs(),
                reason,
            });
        }
    }
    let stuck_total = stuck.len();
    stuck.truncate(NAMED_STUCK_WRITES);
    PublishQueueSnapshot {
        entries: entries.len(),
        outstanding,
        stuck,
        stuck_total,
        unreadable: None,
    }
}

/// Why this entry will not move on its own, if it will not.
///
/// One thing is deliberately NOT here: a write still learning where it goes is
/// not stuck. Route resolution parks deliberately and indefinitely, and calling
/// that stuck would be guessing at exactly what NMP refuses to guess at.
///
/// `SigningState::AwaitingSigner` IS here, and that is new. NMP #1039 named a
/// write parked on a missing signer as the case motivating this door, but the
/// queue projection could not express it: every unsigned state collapsed onto
/// `AwaitingSigner { pubkey }`, so a signature request in flight one
/// millisecond after acceptance was indistinguishable from a key nobody has,
/// and reporting the collapsed state would have made every healthy write
/// momentarily alarming. NMP #1270 split the two: `InFlight` is a signer
/// holding the request right now — transient, normal, ended by the signer
/// answering — and `AwaitingSigner` is nobody answering for that key at all.
/// No clock ends the second one; the person attaching a signer, or the app
/// removing the entry, is its only exit. That is precisely a stuck write.
fn stuck_reason(entry: &PublishQueueEntry) -> Option<String> {
    if let Some(detail) = &entry.persistence_fault {
        return Some(format!(
            "local persistence refused a durable fact: {detail}"
        ));
    }
    match &entry.outcome {
        // Permanently failed and in custody. It ends when the app removes it.
        Some(WriteOutcome::Refused(reason)) => Some(format!("refused at acceptance: {reason:?}")),
        Some(WriteOutcome::NoDestination) => {
            Some("routing finished and named no relays".to_string())
        }
        Some(_) => None,
        None => match &entry.signing {
            SigningState::Refused { reason } => Some(format!("the signer refused: {reason}")),
            SigningState::AwaitingSigner { pubkey } => Some(format!(
                "no signer answers for {pubkey}; attach one or remove the entry"
            )),
            // A signer has it. Nothing to repair and nothing to report.
            SigningState::InFlight { .. } | SigningState::Signed { .. } => None,
        },
    }
}

#[cfg(test)]
#[path = "queue/tests.rs"]
mod tests;
