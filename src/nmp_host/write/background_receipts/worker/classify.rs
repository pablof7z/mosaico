//! Translating one NMP write receipt into Mosaico's progress verdict.
//!
//! Every `WriteStatus` NMP can emit is named here explicitly; the match is
//! exhaustive on purpose so a new NMP receipt fact is a compile error rather
//! than a silently-intermediate status.

use nmp::WriteStatus;

use crate::nmp_host::write::background_receipts::evidence::{
    BackgroundWriteGapStatus, BackgroundWriteTerminalStatus,
};

#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
pub(super) enum ReceiptProgress {
    Intermediate,
    Acked,
    Failure(BackgroundWriteTerminalStatus, String),
    NonterminalFailure(BackgroundWriteTerminalStatus, String),
    Gap(BackgroundWriteGapStatus, String),
}

pub(super) fn classify(status: WriteStatus) -> ReceiptProgress {
    match status {
        WriteStatus::Accepted
        | WriteStatus::Signed(_)
        | WriteStatus::Routed { .. }
        | WriteStatus::AwaitingCapability { .. }
        // Retained, NOT terminal: NMP parks a route indefinitely rather than
        // failing it, so this is progress reporting, not an outcome.
        | WriteStatus::AwaitingRoute { .. }
        | WriteStatus::AwaitingRelay { .. }
        | WriteStatus::AwaitingAuth { .. }
        | WriteStatus::RetryEligible { .. }
        | WriteStatus::HandoffAmbiguous { .. }
        | WriteStatus::Sent { .. } => ReceiptProgress::Intermediate,
        WriteStatus::Acked(_) => ReceiptProgress::Acked,
        WriteStatus::Cancelled => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::Cancelled,
            "write was cancelled before signature promotion".into(),
        ),
        WriteStatus::Failed(reason) => {
            ReceiptProgress::Failure(BackgroundWriteTerminalStatus::Failed, reason)
        }
        WriteStatus::Rejected(relay, reason) => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::Rejected,
            format!("{relay}: {reason}"),
        ),
        WriteStatus::GaveUp(relay) => {
            ReceiptProgress::Failure(BackgroundWriteTerminalStatus::GaveUp, relay.to_string())
        }
        WriteStatus::PersistenceBlocked(relay) => ReceiptProgress::NonterminalFailure(
            BackgroundWriteTerminalStatus::PersistenceBlocked,
            relay.to_string(),
        ),
        WriteStatus::RoutePersistenceBlocked(relay) => ReceiptProgress::NonterminalFailure(
            BackgroundWriteTerminalStatus::RoutePersistenceBlocked,
            relay.to_string(),
        ),
        WriteStatus::OutcomeUnknown(relay) => {
            ReceiptProgress::Gap(BackgroundWriteGapStatus::OutcomeUnknown, relay.to_string())
        }
        // A newer accepted write won the same replaceable coordinate before
        // this one made a wire attempt. Terminal and never retried.
        WriteStatus::Superseded => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::Superseded,
            "a newer accepted write superseded this replaceable coordinate".into(),
        ),
        WriteStatus::AuthDenied {
            relay,
            pubkey,
            source,
            reason,
        } => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::AuthDenied,
            format!("{relay} refused {pubkey} ({source:?}): {reason}"),
        ),
        WriteStatus::ReplaceableConflict { expected, actual } => ReceiptProgress::Failure(
            BackgroundWriteTerminalStatus::ReplaceableConflict,
            format!("expected {expected:?}, actual {actual:?}"),
        ),
    }
}
