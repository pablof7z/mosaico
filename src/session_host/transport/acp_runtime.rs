//! Per-child runtime state for an RPC-hosted session: the captured assistant
//! transcript (so RPC sessions are not blind — defect #6) and the currently
//! running app-server turn id (so mid-turn steer targets the live turn instead
//! of starting a fresh one — defect #3).
//!
//! The [`AcpRuntime`] is fed by a task draining the child's `session/update`
//! stream; the transport reads it when delivering or distilling.

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
    /// Rolling capture of assistant message text, for the status distiller.
    transcript: String,
    /// The id of the turn currently in flight (app-server), if known.
    turn_id: Option<String>,
    /// Whether a turn is believed to be running right now.
    turn_active: bool,
}

impl AcpRuntime {
    /// Fold one inbound notification into the runtime: append any assistant text
    /// and track turn lifecycle. `method`/`params` are the raw notification.
    pub(super) fn note_update(&mut self, method: &str, params: &Value) {
        if let Some(text) = extract_assistant_text(method, params) {
            // Bound the buffer so a very long session cannot grow it without
            // limit; keep the most recent tail the distiller cares about.
            self.transcript.push_str(&text);
            const MAX: usize = 64 * 1024;
            if self.transcript.len() > MAX {
                let cut = self.transcript.len() - MAX;
                self.transcript = self.transcript.split_off(cut);
            }
        }
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

    /// A snapshot of the captured assistant transcript.
    pub(super) fn transcript(&self) -> String {
        self.transcript.clone()
    }
}

/// True for the notifications that end an app-server turn.
fn is_turn_end(method: &str) -> bool {
    matches!(method, "turn/completed" | "turn/failed" | "turn/aborted")
}

/// Pull the turn id out of a notification's params, tolerating the small set of
/// spellings the app-server dialect uses (`turnId` / `turn_id` / `turn.id`).
pub(super) fn extract_turn_id(params: &Value) -> Option<String> {
    for key in ["turnId", "turn_id"] {
        if let Some(s) = params.get(key).and_then(Value::as_str) {
            return Some(s.to_string());
        }
    }
    params
        .get("turn")
        .and_then(|t| t.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Extract assistant-authored text from an update notification. Handles the ACP
/// `session/update` `agent_message_chunk` shape and the common app-server
/// message-delta shapes; returns `None` for tool calls, plans, thoughts, etc.
pub(super) fn extract_assistant_text(method: &str, params: &Value) -> Option<String> {
    if !method.contains("update") && !method.contains("message") && !method.contains("output") {
        return None;
    }
    // ACP: params.update.{sessionUpdate, content:{type:text,text}}.
    if let Some(update) = params.get("update") {
        let kind = update.get("sessionUpdate").and_then(Value::as_str);
        if matches!(kind, Some("agent_message_chunk") | Some("agent_message")) {
            if let Some(t) = content_text(update.get("content")) {
                return Some(t);
            }
        }
        // Some adapters nest the assistant role directly.
        if let Some(t) = role_text(update, "assistant") {
            return Some(t);
        }
    }
    // app-server / generic: params.{content,delta,text} with an assistant role.
    if let Some(t) = role_text(params, "assistant") {
        return Some(t);
    }
    if let Some(t) = content_text(params.get("delta")) {
        return Some(t);
    }
    None
}

/// Read `{role:"<role>", content|text|delta}` text if the role matches.
fn role_text(obj: &Value, want_role: &str) -> Option<String> {
    let role = obj.get("role").and_then(Value::as_str);
    if role.is_some() && role != Some(want_role) {
        return None;
    }
    content_text(obj.get("content"))
        .or_else(|| content_text(obj.get("delta")))
        .or_else(|| obj.get("text").and_then(Value::as_str).map(str::to_string))
}

/// Read text from a `content` value that may be a string, `{type:text,text}`,
/// or an array of such blocks.
fn content_text(content: Option<&Value>) -> Option<String> {
    let content = content?;
    if let Some(s) = content.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    if let Some(t) = content.get("text").and_then(Value::as_str) {
        return (!t.is_empty()).then(|| t.to_string());
    }
    if let Some(arr) = content.as_array() {
        let mut out = String::new();
        for block in arr {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(t) = block.get("text").and_then(Value::as_str) {
                    out.push_str(t);
                }
            } else if let Some(s) = block.as_str() {
                out.push_str(s);
            }
        }
        return (!out.is_empty()).then_some(out);
    }
    None
}

#[cfg(test)]
#[path = "acp_runtime_tests.rs"]
mod acp_runtime_tests;
