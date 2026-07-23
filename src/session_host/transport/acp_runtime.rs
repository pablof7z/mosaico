//! Per-child runtime state for an RPC-hosted session: the currently running
//! app-server turn id, so mid-turn steer targets the live turn instead of
//! starting a fresh one.
//!
//! The [`AcpRuntime`] is fed by a task draining the child's `session/update`
//! stream; the transport reads it when delivering.

use serde_json::Value;

/// How a between/mid-turn steer must be dispatched for an app-server session,
/// derived from the running-turn state. Distinguishing `AwaitingId` from `Idle`
/// is what closes defect #2: a turn that is running but whose id has not yet
/// arrived must NOT be treated as "no turn" (which would start a second
/// concurrent turn) — the steer waits for the id instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SteerState {
    /// A turn is running and its id is known — steer it directly.
    Ready(String),
    /// A turn is running but its id has not been observed yet — gate until known.
    AwaitingId,
    /// No turn is running — a fresh turn may be started.
    Idle,
}

/// Shared, lock-guarded state for one live RPC child.
#[derive(Default)]
pub(super) struct AcpRuntime {
    /// The id of the turn currently in flight (app-server), if known.
    turn_id: Option<String>,
    /// Whether a turn is believed to be running right now.
    turn_active: bool,
}

impl AcpRuntime {
    /// Fold one inbound notification into the runtime and track turn lifecycle.
    pub(super) fn note_update(&mut self, method: &str, params: &Value) {
        if is_turn_end(method) {
            self.turn_active = false;
            self.turn_id = None;
        } else if let Some(id) = extract_turn_id(params) {
            self.turn_id = Some(id);
            self.turn_active = true;
        }
    }

    /// Mark that we just fired a turn/prompt whose id we do not yet know.
    pub(super) fn mark_turn_started(&mut self) {
        self.turn_active = true;
    }

    /// Mark the turn we fired as finished (the fire-and-forget task completed).
    pub(super) fn mark_turn_finished(&mut self) {
        self.turn_active = false;
        self.turn_id = None;
    }

    /// Classify how a steer must be dispatched given the running-turn state.
    pub(super) fn steer_state(&self) -> SteerState {
        match (self.turn_active, &self.turn_id) {
            (true, Some(id)) => SteerState::Ready(id.clone()),
            (true, None) => SteerState::AwaitingId,
            (false, _) => SteerState::Idle,
        }
    }
}

/// True for the notifications that end an app-server turn.
fn is_turn_end(method: &str) -> bool {
    method == "turn/completed"
}

/// Pull the turn id from the current app-server notification shapes.
pub(super) fn extract_turn_id(params: &Value) -> Option<String> {
    if let Some(id) = params.get("turnId").and_then(Value::as_str) {
        return Some(id.to_string());
    }
    params
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
#[path = "acp_runtime_tests.rs"]
mod acp_runtime_tests;
