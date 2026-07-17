use serde_json::Value;

/// The caller-identity fields every in-session RPC sends so the daemon resolves
/// "which session am I" identically. The daemon-side mirror is
/// `CallerAnchor::from_params`.
pub(crate) struct InvocationContext {
    session: Option<String>,
    pty_session: Option<String>,
    harness: Option<&'static str>,
    watch_pid: Option<i32>,
    agent: Option<String>,
    cwd: Option<String>,
    group: Option<String>,
}

impl InvocationContext {
    pub(crate) fn from_current_process() -> Self {
        let session = std::env::var("MOSAICO_PUBKEY")
            .ok()
            .filter(|value| !value.is_empty());
        let pty_session = super::pty_session_env();
        let admitted_harness = std::env::var("MOSAICO_OBSERVED_HARNESS")
            .ok()
            .filter(|value| !value.is_empty())
            .and_then(|value| match crate::session::Harness::from_str(&value) {
                crate::session::Harness::Unknown => None,
                harness => Some(harness.as_str()),
            });
        let watch_anchor = if pty_session.is_none() && admitted_harness.is_none() {
            super::hooks::caller_watch_pid_anchor()
        } else {
            None
        };
        let (harness, watch_pid) = admitted_harness.map_or_else(
            || {
                watch_anchor
                    .map(|(harness, pid)| (Some(harness), Some(pid)))
                    .unwrap_or((None, None))
            },
            |harness| (Some(harness), None),
        );
        Self {
            session,
            pty_session,
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
            "session": self.session,
            "pty_session": self.pty_session,
            "harness": self.harness,
            "watch_pid": self.watch_pid,
            "agent": self.agent,
            "cwd": self.cwd,
            "group": self.group,
        })
    }
}

pub(crate) fn merge_rpc_params(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            if value.is_null() && base.get(key).is_some_and(|current| !current.is_null()) {
                continue;
            }
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_optional_selector_does_not_erase_caller_pubkey() {
        let params = merge_rpc_params(
            serde_json::json!({"session": "pubkey", "cwd": "/work"}),
            serde_json::json!({"session": null, "cwd": "/override"}),
        );
        assert_eq!(params["session"], "pubkey");
        assert_eq!(params["cwd"], "/override");
    }
}
