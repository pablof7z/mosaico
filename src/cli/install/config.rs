//! Harness discovery and hook entry configuration.

use anyhow::{bail, Result};
use std::path::PathBuf;

pub const OPENCODE_PLUGIN_TS: &str = include_str!("../../../integrations/opencode/tenex-edge.ts");

#[derive(Debug)]
pub struct Harness {
    pub id: &'static str,
    pub display: &'static str,
    pub config_path: PathBuf,
    pub detected: bool,
}

pub fn harnesses() -> Result<Vec<Harness>> {
    let home = home_dir()?;
    Ok(vec![
        Harness {
            id: "claude-code",
            display: "Claude Code",
            config_path: home.join(".claude/settings.json"),
            detected: home.join(".claude").exists() || bin_on_path("claude"),
        },
        Harness {
            id: "codex",
            display: "Codex",
            config_path: home.join(".codex/hooks.json"),
            detected: home.join(".codex").exists() || bin_on_path("codex"),
        },
        Harness {
            id: "opencode",
            display: "opencode",
            config_path: home.join(".config/opencode/plugin/tenex-edge.ts"),
            detected: home.join(".config/opencode").exists() || bin_on_path("opencode"),
        },
        Harness {
            id: "grok",
            display: "Grok Build",
            config_path: home.join(".grok/user-settings.json"),
            detected: home.join(".grok").exists() || bin_on_path("grok"),
        },
    ])
}

pub(super) fn home_dir() -> Result<PathBuf> {
    home_dir_from_env(std::env::var("HOME").ok())
}

fn home_dir_from_env(home: Option<String>) -> Result<PathBuf> {
    let Some(home) = home.filter(|h| !h.is_empty()) else {
        bail!(
            "HOME is not set: refusing to install harness hooks under the current directory. \
             Set HOME to the real user home; TENEX_EDGE_HOME only controls tenex-edge daemon state."
        );
    };
    Ok(PathBuf::from(home))
}

pub(super) fn claude_detected() -> Result<bool> {
    let home = home_dir()?;
    Ok(home.join(".claude").exists() || bin_on_path("claude"))
}

fn bin_on_path(bin: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

/// The hook signature we dedupe by: `tenex-edge harness hook <host> --type <type>`.
fn sig(host: &str, ty: &str) -> String {
    format!("tenex-edge harness hook {host} --type {ty}")
}

fn claude_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
    let mk = |ty: &str, timeout: u64| {
        serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": sig("claude-code", ty),
                "timeout": timeout,
            }]
        })
    };
    vec![
        ("SessionStart", mk("session-start", 60)),
        ("SessionEnd", mk("session-end", 30)),
        ("UserPromptSubmit", mk("user-prompt-submit", 30)),
        ("PostToolUse", mk("post-tool-use", 10)),
        ("Stop", mk("stop", 10)),
    ]
}

pub fn codex_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
    let mk = |ty: &str, timeout: u64, matcher: Option<&str>| {
        let mut entry = serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": sig("codex", ty),
                "timeout": timeout,
            }]
        });
        if let Some(m) = matcher {
            entry["matcher"] = serde_json::Value::String(m.into());
        }
        entry
    };
    vec![
        (
            "SessionStart",
            mk("session-start", 60, Some("startup|resume")),
        ),
        ("UserPromptSubmit", mk("user-prompt-submit", 30, None)),
        ("PostToolUse", mk("post-tool-use", 10, None)),
        ("Stop", mk("stop", 30, None)),
    ]
}

fn grok_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
    let mk = |ty: &str, timeout: u64| {
        serde_json::json!({
            "hooks": [{
                "type": "command",
                "command": sig("grok", ty),
                "timeout": timeout,
            }]
        })
    };
    vec![
        ("SessionStart", mk("session-start", 60)),
        ("SessionEnd", mk("session-end", 10)),
        ("UserPromptSubmit", mk("user-prompt-submit", 30)),
        ("PostToolUse", mk("post-tool-use", 10)),
        ("Stop", mk("stop", 10)),
    ]
}

pub fn hook_entries(h: &Harness) -> Vec<(&'static str, serde_json::Value)> {
    match h.id {
        "claude-code" => claude_hook_entries(),
        "codex" => codex_hook_entries(),
        "grok" => grok_hook_entries(),
        _ => Vec::new(),
    }
}

pub fn host_for_harness(h: &Harness) -> &'static str {
    match h.id {
        "claude-code" => "claude-code",
        "codex" => "codex",
        "grok" => "grok",
        _ => h.id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_dir_uses_home_env() {
        assert_eq!(
            home_dir_from_env(Some("/Users/alice".to_string())).unwrap(),
            PathBuf::from("/Users/alice")
        );
    }

    #[test]
    fn home_dir_refuses_absent_or_empty_home() {
        for home in [None, Some(String::new())] {
            let err = home_dir_from_env(home).unwrap_err().to_string();
            assert!(err.contains("HOME is not set"));
            assert!(err.contains("TENEX_EDGE_HOME only controls"));
        }
    }
}
