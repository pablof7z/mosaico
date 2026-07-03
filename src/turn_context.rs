//! Agent turn-context assembly shared by daemon RPCs and hook tests.
//!
//! The CLI owns hook I/O and rendering commands. The daemon owns turn state.
//! This module owns the shared text/audit assembly used between them.

mod check;
mod reads;
mod start;

pub(crate) use check::assemble_turn_check;
#[cfg(test)]
pub(crate) use check::assemble_turn_check_context;
pub(crate) use start::assemble_turn_start;
#[cfg(test)]
pub(crate) use start::assemble_turn_start_context;

use crate::reconcile::HookContextReceipt;

/// One turn's assembled fabric snapshot plus its graph-sourced receipt. The text
/// is what the agent sees (suppressed to `None` when empty); the receipt is the
/// render's OWN dependency trace — it REPLACES the hand-rolled `turn_start_audit`
/// / `turn_check_audit` and cannot drift from the bytes.
pub(crate) struct TurnContext {
    pub(crate) text: Option<String>,
    pub(crate) receipt: HookContextReceipt,
}

#[cfg(test)]
mod tests;
