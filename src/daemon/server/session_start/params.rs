#[derive(serde::Deserialize, Default)]
pub(super) struct SessionStartParams {
    pub(super) agent: String,
    /// Real argv of a direct `claude --agent <slug>` invocation, detected by
    /// the hook when TENEX_EDGE_AGENT was absent. Seeds a brand-new agent's
    /// spawn command; ignored when the agent already exists.
    #[serde(default)]
    pub(super) provision_command: Option<Vec<String>>,
    /// Harness-native external session id. This is only a `session_aliases`
    /// locator, never the identity.
    #[serde(default)]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) watch_pid: Option<i32>,
    /// Portable-pty supervisor session id from TENEX_EDGE_PTY_SESSION.
    #[serde(default)]
    pub(super) pty_session: Option<String>,
    /// Daemon-owned reservation acquired before a manual PTY launch.
    #[serde(default)]
    pub(super) durable_reservation: Option<String>,
    /// Operator-selected public handle prefix from `launch --name`.
    #[serde(default)]
    pub(super) session_name: Option<String>,
    /// Harness-native resume token. Opencode forwards its `ses_*` id here.
    #[serde(default)]
    pub(super) resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`).
    #[serde(default)]
    pub(super) harness: Option<String>,
    /// NIP-29 channel (`h`) this hosted process was spawned into.
    #[serde(default)]
    pub(super) channel: Option<String>,
    /// Full channel set this hosted process should join. `channel` is the active
    /// channel; this list drives status h-tags/inbox scope.
    #[serde(default)]
    pub(super) channels: Vec<String>,
    /// Dispatch kind:9 event id that caused this hosted process to start.
    #[serde(default)]
    pub(super) dispatch_event: Option<String>,
}
