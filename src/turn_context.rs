//! Agent turn-context assembly shared by daemon RPCs and hook tests.
//!
//! The CLI owns hook I/O and rendering commands. The daemon owns turn state.
//! This module owns the shared text/audit assembly used between them.

mod check;
mod distill_notice;
mod reads;
mod start;

pub(crate) use check::assemble_turn_check;
#[cfg(test)]
pub(crate) use check::assemble_turn_check_context;
pub(crate) use start::assemble_turn_start;
#[cfg(test)]
pub(crate) use start::assemble_turn_start_context;

use crate::fabric_context::ViewInputs;
use crate::reconcile::{
    HookContextOutcome, HookContextReceipt, HookContextReconciler, HookContextRenderFact, InputFact,
};
use std::collections::HashMap;
use std::sync::Mutex;

/// Daemon-held, per-session hook-context graphs. Test shims may construct a
/// local map, but production render paths reuse the daemon-owned instance.
pub(crate) type HookContextGraphs = Mutex<HashMap<String, HookContextReconciler>>;

pub(crate) fn render_hook_context(
    graphs: &HookContextGraphs,
    session_id: &str,
    kind: &str,
    cursor: i64,
    now: i64,
    inputs: ViewInputs,
) -> trellis_core::GraphResult<HookContextOutcome> {
    let mut guard = graphs.lock().expect("hook-context mutex poisoned");
    let graph = guard.entry(session_id.to_string()).or_default();
    graph.render_context(session_id, kind, cursor, now, inputs)
}

/// One turn's assembled fabric snapshot plus its graph-sourced receipt. The text
/// is what the agent sees (suppressed to `None` when empty); the receipt is the
/// render's OWN dependency trace — it REPLACES the hand-rolled `turn_start_audit`
/// / `turn_check_audit` and cannot drift from the bytes.
pub(crate) struct TurnContext {
    pub(crate) text: Option<String>,
    pub(crate) receipt: HookContextReceipt,
    /// The render's committed transaction id / revision, for the receipts ledger.
    pub(crate) transaction_id: i64,
    pub(crate) revision: i64,
    pub(crate) replay_fact: Option<InputFact>,
}

pub(crate) fn hook_replay_fact(
    session_id: &str,
    hook_kind: &str,
    cursor: i64,
    now: i64,
    force: bool,
    inputs: &ViewInputs,
    text: Option<&str>,
) -> Option<InputFact> {
    let inputs_json = match serde_json::to_value(inputs) {
        Ok(value) => value,
        Err(e) => {
            tracing::warn!(session = %session_id, error = %e, "hook replay capsule serialization failed");
            return None;
        }
    };
    Some(InputFact::HookContextRender(HookContextRenderFact {
        session_id: session_id.to_string(),
        hook_kind: hook_kind.to_string(),
        cursor,
        now,
        force,
        emitted_text_hash: text.map(crate::replay_capsules::text_hash),
        inputs_json,
    }))
}

#[cfg(test)]
mod tests;
