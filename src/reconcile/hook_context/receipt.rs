//! The plain, Trellis-FREE receipt the daemon persists and the `explain` CLI can
//! replay. It is sourced from `why_output_frame` / `why_changed` — the render's
//! OWN dependency trace — so it CANNOT drift from the snapshot the way the old
//! hand-rolled `turn_start_audit` did: the explanation IS the derivation.

use trellis_core::OutputFrameKind;

/// The full-vs-delta shape, decided purely by the `cursor` input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// `cursor == 0`: full `<members>` / `<subchannels>` roster.
    Full,
    /// `cursor > 0`: delta `<recent-presence>` only.
    Delta,
}

impl Shape {
    fn from_cursor(cursor: i64) -> Self {
        if cursor == 0 {
            Shape::Full
        } else {
            Shape::Delta
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Shape::Full => "full",
            Shape::Delta => "delta",
        }
    }
}

/// The output-frame kind that carried this snapshot, mirrored as plain data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameKind {
    /// First snapshot for the graph.
    Baseline,
    /// A change delta over the previous snapshot.
    Delta,
    /// A forced discontinuity.
    Rebaseline,
    /// The snapshot was cleared.
    Clear,
    /// Nothing was emitted (inputs unchanged).
    Unchanged,
}

impl FrameKind {
    pub(super) fn from_output_kind(kind: Option<&OutputFrameKind>) -> Self {
        match kind {
            Some(OutputFrameKind::Baseline(_)) => FrameKind::Baseline,
            Some(OutputFrameKind::Delta(_)) => FrameKind::Delta,
            Some(OutputFrameKind::Rebaseline(..)) => FrameKind::Rebaseline,
            Some(OutputFrameKind::Clear(_)) => FrameKind::Clear,
            None => FrameKind::Unchanged,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            FrameKind::Baseline => "baseline",
            FrameKind::Delta => "delta",
            FrameKind::Rebaseline => "rebaseline",
            FrameKind::Clear => "clear",
            FrameKind::Unchanged => "unchanged",
        }
    }
}

/// Graph-sourced receipt for one hook-context render. Everything here is derived
/// from the SAME dependency trace that produced the bytes, so it answers "why is
/// @X shown as working / why is #Y not-joined / why this shape" without any
/// re-derivation that could diverge from the render.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookContextReceipt {
    /// Instrumentation surface tag (`hook_context`).
    pub surface: &'static str,
    /// Hook kind label the daemon assigns (`turn_start` / `turn_check`).
    pub kind: String,
    /// The session this snapshot was rendered for.
    pub session_id: String,
    /// The explicit seen-cursor input.
    pub cursor: i64,
    /// The explicit wall-clock input.
    pub now: i64,
    /// Full vs delta, attributable to the `cursor` input.
    pub shape: Shape,
    /// The output-frame kind that carried this snapshot.
    pub frame: FrameKind,
    /// Whether a snapshot was injected (non-empty and/or forced).
    pub emitted: bool,
    /// Rendered snapshot byte length (0 when suppressed).
    pub bytes: usize,
    /// Canonical input labels the graph attributes this frame to (e.g. `presence`,
    /// `cursor`, `now`) — the "why did the snapshot change" answer.
    pub input_causes: Vec<String>,
}

impl HookContextReceipt {
    pub(super) fn new(
        kind: &str,
        session_id: &str,
        cursor: i64,
        now: i64,
        frame: FrameKind,
        text: Option<&str>,
        input_causes: Vec<String>,
    ) -> Self {
        Self {
            surface: "hook_context",
            kind: kind.to_string(),
            session_id: session_id.to_string(),
            cursor,
            now,
            shape: Shape::from_cursor(cursor),
            frame,
            emitted: text.is_some(),
            bytes: text.map(str::len).unwrap_or(0),
            input_causes,
        }
    }

    /// Serialize for the hook-call log / `receipts` store. Carries `kind` so the
    /// debug loader keeps labelling turn_start / turn_check calls.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "surface": self.surface,
            "kind": self.kind,
            "session_id": self.session_id,
            "cursor": self.cursor,
            "now": self.now,
            "shape": self.shape.as_str(),
            "frame": self.frame.as_str(),
            "why_shape": format!(
                "cursor {} 0 → {} render",
                if self.cursor == 0 { "==" } else { ">" },
                self.shape.as_str()
            ),
            "input_causes": self.input_causes,
            "output": {
                "emitted": self.emitted,
                "bytes": self.bytes,
            },
        })
    }
}
