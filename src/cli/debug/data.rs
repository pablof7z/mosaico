use std::time::Duration;

pub struct HookTailOpts {
    pub roots: Vec<String>,
    pub session: Option<String>,
    pub panes: usize,
    pub refresh: Duration,
}

#[derive(Clone, Debug)]
pub struct DebugLine {
    pub ts_ms: u128,
    pub kind: DebugKind,
    pub label: String, // event type, e.g. "user-prompt-submit", "inject", "inbox send"
    pub summary: String, // smart one-liner shown in the timeline
    pub detail: String, // full content for the detail panel (real newlines)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugKind {
    Hook,
    Inject,
    Command,
    Error,
    Session,
}

#[derive(Clone, Debug, Default)]
pub struct SessionPane {
    pub session: String,
    pub short: String,
    pub root: String,
    pub agent: String,
    pub host: String,
    pub lines: Vec<DebugLine>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct HookTailSnapshot {
    pub panes: Vec<SessionPane>,
    pub unscoped: Vec<DebugLine>,
    pub roots: Vec<String>,
    pub sessions: Vec<String>,
}

pub struct RootPopup {
    pub cursor: usize,
}
