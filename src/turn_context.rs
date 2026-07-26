//! Agent turn-context assembly shared by daemon RPCs and hook tests.
//!
//! The CLI owns hook I/O and rendering commands. The daemon owns turn state.
//! This module owns the shared text/audit assembly used between them.

mod check;
mod headless;
pub(crate) mod history;
mod reads;
mod start;

pub(crate) use check::assemble_turn_check;
#[cfg(test)]
pub(crate) use check::assemble_turn_check_context;
pub(crate) use start::assemble_turn_start;
#[cfg(test)]
pub(crate) use start::render_turn_start_text_for_test;

use crate::fabric_context::ViewInputs;
use crate::reconcile::{HookContextOutcome, HookContextReceipt, HookContextState};
use std::collections::HashMap;
use std::sync::Mutex;

/// Daemon-held, per-session hook-context states. Tests may construct a
/// local map, but production render paths reuse the daemon-owned instance.
pub(crate) type HookContextStates = Mutex<HashMap<String, HookContextState>>;

pub(crate) fn render_hook_context(
    states: &HookContextStates,
    pubkey: &str,
    kind: &str,
    cursor: i64,
    now: i64,
    inputs: ViewInputs,
) -> HookContextOutcome {
    let mut guard = states.lock().expect("hook-context mutex poisoned");
    let state = guard.entry(pubkey.to_string()).or_default();
    state.render_context(pubkey, kind, cursor, now, inputs)
}

/// One turn's assembled fabric snapshot plus its state-produced receipt. The text
/// is what the agent sees (suppressed to `None` when empty); the receipt is
/// calculated by the same render and cannot drift from the bytes.
pub(crate) struct TurnContext {
    pub(crate) text: Option<String>,
    pub(crate) receipt: HookContextReceipt,
    /// Monotonic render identifiers for the receipts ledger.
    pub(crate) transaction_id: i64,
    pub(crate) revision: i64,
    /// Roster pubkeys omitted for lacking a resolvable kind:0 handle. The daemon
    /// RPC handler schedules a debounced refetch for them.
    pub(crate) missing_profiles: Vec<String>,
}

impl TurnContext {
    /// Add a daemon policy advisory after the ordinary fabric view has rendered.
    /// Keep the persisted receipt aligned with the exact bytes the hook emits.
    pub(crate) fn append_advisory(&mut self, advisory: &str, cause: &str) {
        let text = match self.text.take() {
            Some(existing) => format!("{existing}\n\n{advisory}"),
            None => advisory.to_string(),
        };
        self.receipt.emitted = true;
        self.receipt.bytes = text.len();
        if !self.receipt.input_causes.iter().any(|item| item == cause) {
            self.receipt.input_causes.push(cause.to_string());
        }
        self.text = Some(text);
    }
}

#[cfg(test)]
mod tests;
