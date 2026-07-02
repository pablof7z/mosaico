use serde_json::Value;

/// The caller-identity fields every in-session RPC sends so the daemon resolves
/// "which session am I" identically. The daemon-side mirror is
/// `CallerAnchor::from_params`.
pub(crate) struct InvocationContext {
    tmux_pane: Option<String>,
    harness: Option<&'static str>,
    watch_pid: Option<i32>,
    agent: Option<String>,
    cwd: Option<String>,
    group: Option<String>,
}

impl InvocationContext {
    pub(crate) fn from_current_process() -> Self {
        let tmux_pane = super::tmux_pane_env();
        let watch_anchor = if tmux_pane.is_none() {
            super::hooks::caller_watch_pid_anchor()
        } else {
            None
        };
        let (harness, watch_pid) = watch_anchor
            .map(|(harness, pid)| (Some(harness), Some(pid)))
            .unwrap_or((None, None));
        Self {
            tmux_pane,
            harness,
            watch_pid,
            agent: super::agent_env_slug(),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().to_string()),
            group: super::channel_env(),
        }
    }

    pub(crate) fn to_rpc_json(&self) -> Value {
        serde_json::json!({
            "tmux_pane": self.tmux_pane,
            "harness": self.harness,
            "watch_pid": self.watch_pid,
            "agent": self.agent,
            "cwd": self.cwd,
            "group": self.group,
        })
    }
}
