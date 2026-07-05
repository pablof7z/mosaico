#[derive(serde::Deserialize, Default)]
pub(super) struct SessionStartParams {
    pub(super) agent: String,
    /// Harness-native external session id. Hooks send `harness_session_id`; the
    /// legacy/CLI path sends `session_id`. This is only a `session_aliases`
    /// locator, never the identity.
    #[serde(default, alias = "harness_session_id")]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) watch_pid: Option<i32>,
    /// Stable tmux pane id from $TMUX_PANE, e.g. "%5".
    #[serde(default)]
    pub(super) tmux_pane: Option<String>,
    /// Value of $TMUX: socket path, session id, pane id.
    #[serde(default)]
    pub(super) tmux_socket: Option<String>,
    /// Harness-native resume token. Opencode forwards its `ses_*` id here.
    #[serde(default)]
    pub(super) resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`).
    #[serde(default)]
    pub(super) harness: Option<String>,
    /// NIP-29 channel (`h`) this pane was spawned into.
    #[serde(default)]
    pub(super) channel: Option<String>,
    /// Exact ordinal to allocate for this session.
    #[serde(default)]
    pub(super) preferred_ordinal: Option<u32>,
}
