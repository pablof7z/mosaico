#[derive(serde::Deserialize, Default)]
pub(super) struct SessionStartParams {
    pub(super) agent: String,
    /// Real argv of a direct `claude --agent <slug>` invocation, detected by
    /// the hook when TENEX_EDGE_AGENT was absent. Seeds a brand-new agent's
    /// spawn command; ignored when the agent already exists.
    #[serde(default)]
    pub(super) provision_command: Option<Vec<String>>,
    /// Harness-native external session id. Hooks send `harness_session_id`; the
    /// legacy/CLI path sends `session_id`. This is only a `session_aliases`
    /// locator, never the identity.
    #[serde(default, alias = "harness_session_id")]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) watch_pid: Option<i32>,
    /// Portable-pty supervisor session id from TENEX_EDGE_PTY_SESSION.
    #[serde(default)]
    pub(super) pty_session: Option<String>,
    /// Portable-pty supervisor socket path from TENEX_EDGE_PTY_SOCKET.
    #[serde(default)]
    pub(super) pty_socket: Option<String>,
    /// Harness-native resume token. Opencode forwards its `ses_*` id here.
    #[serde(default)]
    pub(super) resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`).
    #[serde(default)]
    pub(super) harness: Option<String>,
    /// NIP-29 channel (`h`) this hosted process was spawned into.
    #[serde(default)]
    pub(super) channel: Option<String>,
}
