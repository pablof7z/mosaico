mod args;
mod data;
mod loader;
mod render;
mod state;
mod util;

// Re-export public items for cli.rs to use
pub(super) use args::{debug, DebugAction};
pub use data::HookTailOpts;

// Wrapper to match the original signature
pub(super) fn hook_tail(opts: HookTailOpts) -> anyhow::Result<()> {
    state::hook_tail(opts)
}

#[cfg(test)]
mod tests {
    use super::data::SessionPane;
    use super::util::*;
    use crate::test_env::EnvGuard;
    use std::collections::BTreeMap;

    #[test]
    fn command_session_uses_explicit_flag_only() {
        let v = serde_json::json!({
            "env": {"MOSAICO_PUBKEY": "env-pubkey"},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));

        let v = serde_json::json!({
            "env": {},
            "command": {"explicit_session": "flag-session"},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v).as_deref(), Some("flag-session"));

        let v = serde_json::json!({
            "env": {"MOSAICO_PUBKEY": "env-pubkey"},
            "command": {},
            "process": {"cwd": "/tmp"}
        });
        assert_eq!(command_session(&v), None);
    }

    #[test]
    fn hook_session_accepts_canonical_session_id() {
        let v = serde_json::json!({"session_id": "codex-session"});
        assert_eq!(hook_session(&v).as_deref(), Some("codex-session"));
    }

    #[test]
    fn command_session_can_infer_unique_live_agent_channel() {
        let mut panes = BTreeMap::new();
        panes.insert(
            "session-a".to_string(),
            SessionPane {
                session: "session-a".to_string(),
                root: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(
            infer_command_session(&panes, "coder", "proj").as_deref(),
            Some("session-a")
        );

        panes.insert(
            "session-b".to_string(),
            SessionPane {
                session: "session-b".to_string(),
                root: "proj".to_string(),
                agent: "coder".to_string(),
                ..SessionPane::default()
            },
        );
        assert_eq!(infer_command_session(&panes, "coder", "proj"), None);
    }

    #[test]
    fn hook_telemetry_from_unknown_workspace_is_unscoped() {
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("MOSAICO_HOME", home.path());
        let mut pane = SessionPane::default();
        let hook = serde_json::json!({"cwd": "/definitely/not/a/mosaico-workspace"});

        fill_pane_from_hook(&mut pane, "codex", &hook).unwrap();

        assert_eq!(pane.host, "codex");
        assert!(pane.root.is_empty());
    }

    #[test]
    fn command_telemetry_from_unknown_workspace_is_unscoped() {
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("MOSAICO_HOME", home.path());
        let command = serde_json::json!({
            "process": {"cwd": "/definitely/not/a/mosaico-workspace"}
        });

        assert_eq!(command_root(&command).unwrap(), "");
    }

    #[test]
    fn corrupt_workspace_map_still_fails_telemetry_loading() {
        let home = tempfile::tempdir().unwrap();
        let _env = EnvGuard::set("MOSAICO_HOME", home.path());
        std::fs::write(home.path().join("workspaces.json"), "{not json").unwrap();
        let command = serde_json::json!({
            "process": {"cwd": "/definitely/not/a/mosaico-workspace"}
        });

        let error = command_root(&command).unwrap_err();
        assert!(
            format!("{error:#}").contains("corrupt workspace map"),
            "error = {error:#}"
        );
    }
}
