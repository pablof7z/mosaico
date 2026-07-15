#[derive(serde::Deserialize, Default)]
pub(super) struct SessionStartParams {
    pub(super) agent: String,
    #[serde(default)]
    pub(super) profile: Option<String>,
    /// Authoritative pubkey allocated before a managed process is spawned.
    #[serde(default)]
    pub(super) pubkey: Option<String>,
    #[serde(default)]
    pub(super) reclaimed_pubkey: Option<String>,
    #[serde(default)]
    pub(super) harness_session: Option<String>,
    #[serde(default)]
    pub(super) cwd: Option<String>,
    #[serde(default)]
    pub(super) watch_pid: Option<i32>,
    #[serde(default)]
    pub(super) pty_session: Option<String>,
    #[serde(default)]
    pub(super) endpoint_kind: Option<String>,
    #[serde(default)]
    pub(super) session_name: Option<String>,
    #[serde(default)]
    pub(super) resume_id: Option<String>,
    #[serde(default)]
    pub(super) harness: Option<String>,
    #[serde(default)]
    pub(super) channel: Option<String>,
    #[serde(default)]
    pub(super) channels: Vec<String>,
    #[serde(default)]
    pub(super) dispatch_event: Option<String>,
}

pub(super) fn resolve_harness(p: &SessionStartParams) -> crate::session::Harness {
    use crate::session::Harness;
    p.harness
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(Harness::from_str)
        .unwrap_or_else(|| {
            if p.resume_id.is_some() {
                Harness::Opencode
            } else if p.harness_session.is_some() {
                Harness::ClaudeCode
            } else {
                Harness::Unknown
            }
        })
}
