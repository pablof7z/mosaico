//! What one write's relay lanes have said, and what its outcome means.
//!
//! NMP's publish-queue vocabulary already answers "is this over?" — the whole
//! write ends on exactly one [`WriteOutcome`], and [`RelayState::is_terminal`]
//! answers it per relay. Nothing here re-derives that. What is left is the one
//! judgement that is genuinely Mosaico's: which of these facts an operator
//! should be shown, and under what name.

use nmp::{
    NotSentReason, RefuseReason, RelayState, RelayUrl, RelayWaiting, SigningState, WriteOutcome,
};

use super::super::evidence::{BackgroundWriteGapStatus, BackgroundWriteTerminalStatus};
use super::StreamOutcome;

/// The per-relay facts observed on one receipt stream so far.
#[derive(Default)]
pub(in crate::nmp_host::write::background_receipts) struct LaneFacts {
    /// At least one relay acked. A four-relay publish where one relay is given
    /// up on and three published is a success with a footnote, so this alone
    /// decides the stream's verdict; the footnotes are filed as they arrive.
    published: bool,
    /// The most recent relay fault, kept so a write that reaches NO relay can
    /// name why rather than reporting a bare silence.
    fault: Option<(BackgroundWriteTerminalStatus, String)>,
}

impl LaneFacts {
    /// Record one relay fact, returning the fault an operator should see.
    ///
    /// A fault is reported the moment it is observed and never withheld until
    /// the write ends: `PersistenceStalled` in particular must survive a later
    /// ack, because an operator must not lose the only signal that the local
    /// disk is failing just because a relay accepted the event afterwards.
    pub(super) fn observe_relay(
        &mut self,
        relay: &RelayUrl,
        state: RelayState,
    ) -> Option<(BackgroundWriteTerminalStatus, String)> {
        if matches!(state, RelayState::Published) {
            self.published = true;
        }
        let fault = relay_fault(relay, state)?;
        self.fault = Some(fault.clone());
        Some(fault)
    }

    /// Turn the whole-write terminal into this stream's verdict.
    pub(super) fn settle(&mut self, outcome: WriteOutcome) -> StreamOutcome {
        match outcome {
            WriteOutcome::Settled => match (self.published, self.fault.take()) {
                (true, _) => StreamOutcome::Acked,
                (false, Some((status, detail))) => StreamOutcome::Failure(status, detail),
                // The destination set was closed and every member terminal, so
                // a per-relay fact explaining this existed — it just never
                // reached this observer. That is a hole in what we saw, not a
                // verdict about the write.
                (false, None) => StreamOutcome::Gap(
                    BackgroundWriteGapStatus::ReceiptDisconnected,
                    "the write settled without a per-relay fact reaching this observer".into(),
                ),
            },
            WriteOutcome::NoDestination => StreamOutcome::Failure(
                BackgroundWriteTerminalStatus::NoDestination,
                "routing finished and named no relays".into(),
            ),
            WriteOutcome::NotSent(NotSentReason::Cancelled) => StreamOutcome::Failure(
                BackgroundWriteTerminalStatus::Cancelled,
                "write was cancelled before signature promotion".into(),
            ),
            WriteOutcome::NotSent(NotSentReason::Superseded) => StreamOutcome::Superseded,
            WriteOutcome::Refused(reason) => StreamOutcome::Failure(
                BackgroundWriteTerminalStatus::Refused,
                refusal_detail(&reason),
            ),
        }
    }
}

/// The reason a signature will never arrive, if the signing state says so.
pub(super) fn signer_refusal(signing: SigningState) -> Option<String> {
    match signing {
        // A signer holding the request, a signer that is simply not attached
        // yet, or a signature that already exists. None of the three is an
        // outcome, and no clock ends the middle one -- which is why the
        // durable queue projection, not this bounded observation, is where a
        // permanently parked write gets named (`super::super::super::queue`).
        SigningState::InFlight { .. }
        | SigningState::AwaitingSigner { .. }
        | SigningState::Signed { .. } => None,
        SigningState::Refused { reason } => Some(reason),
    }
}

/// The operator-visible fault in one relay fact, if there is one.
fn relay_fault(
    relay: &RelayUrl,
    state: RelayState,
) -> Option<(BackgroundWriteTerminalStatus, String)> {
    match state {
        RelayState::Waiting(RelayWaiting::PersistenceStalled { detail }) => Some((
            BackgroundWriteTerminalStatus::PersistenceStalled,
            format!("{relay}: {detail}"),
        )),
        // Not connected yet, waiting on AUTH, or backing off with a stated
        // cause. Every one of these is an ordinary lane state, and offline
        // time spends no attempt.
        RelayState::Waiting(_) | RelayState::Sent { .. } | RelayState::Published => None,
        RelayState::Rejected { reason } => Some((
            BackgroundWriteTerminalStatus::Rejected,
            format!("{relay} refused the event: {reason}"),
        )),
        // Deliberately NOT `Rejected`. The relay authenticating the identity
        // and then refusing this event is a different repair from the app's
        // own policy declining to authenticate, the signer declining to sign
        // the challenge, or the relay refusing the identity outright.
        RelayState::AuthFailed {
            pubkey,
            source,
            reason,
        } => Some((
            BackgroundWriteTerminalStatus::AuthFailed,
            format!("{relay} could not authenticate {pubkey} ({source:?}): {reason}"),
        )),
        RelayState::GaveUp => Some((
            BackgroundWriteTerminalStatus::GaveUp,
            format!("{relay}: the publish attempt ceiling was reached"),
        )),
    }
}

/// The store's semantic no, rendered so the repair is legible.
fn refusal_detail(reason: &RefuseReason) -> String {
    match reason {
        RefuseReason::AlreadyExpired => {
            "the event's NIP-40 expiration had already passed at acceptance".into()
        }
        RefuseReason::Tombstoned => {
            "the event or its address was tombstoned by a verified deletion".into()
        }
        // Both ids are kept because they make the failure recoverable without
        // troubling anyone: fetch `actual`, reapply the change, resubmit.
        RefuseReason::ReplaceableBaseChanged { expected, actual } => format!(
            "the replaceable base moved (expected {expected:?}, actual {actual:?}); \
             fetch the actual event, reapply the change and resubmit"
        ),
        RefuseReason::ReplaceableBaseOnRegularEvent => {
            "a replaceable-base precondition was attached to a kind with no replaceable coordinate"
                .into()
        }
    }
}
