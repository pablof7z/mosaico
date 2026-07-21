//! Harness discovery and hook entry configuration.

use anyhow::{bail, Result};
use std::path::PathBuf;

pub const OPENCODE_PLUGIN_TS: &str = include_str!("../../../integrations/opencode/mosaico.ts");
pub const HERMES_PLUGIN_YAML: &str = include_str!("../../../integrations/hermes/plugin.yaml");
pub const HERMES_PLUGIN_PY: &str = include_str!("../../../integrations/hermes/__init__.py");

#[derive(Debug)]
pub struct Harness {
    pub id: &'static str,
    pub display: &'static str,
    pub config_path: PathBuf,
    pub detected: bool,
}

pub fn harnesses() -> Result<Vec<Harness>> {
    let home = home_dir()?;
    let grok_home = grok_home_dir(std::env::var("GROK_HOME").ok(), &home);
    let hermes_home = hermes_home_dir(std::env::var("HERMES_HOME").ok(), &home);
    let available = crate::config::detect_available_harnesses()?;
    Ok(vec![
        Harness {
            id: "claude-code",
            display: "Claude Code",
            config_path: home.join(".claude/settings.json"),
            detected: available.contains(&crate::session::Harness::ClaudeCode),
        },
        Harness {
            id: "codex",
            display: "Codex",
            config_path: home.join(".codex/hooks.json"),
            detected: available.contains(&crate::session::Harness::Codex),
        },
        Harness {
            id: "opencode",
            display: "opencode",
            config_path: home.join(".config/opencode/plugin/mosaico.ts"),
            detected: available.contains(&crate::session::Harness::Opencode),
        },
        Harness {
            id: "grok",
            display: "Grok Build",
            config_path: grok_home.join("hooks/mosaico.json"),
            detected: available.contains(&crate::session::Harness::Grok),
        },
        Harness {
            id: "goose",
            display: "Goose",
            config_path: crate::goose_integration::plugin_root()?,
            detected: available.contains(&crate::session::Harness::Goose),
        },
        Harness {
            id: "hermes",
            display: "Hermes Agent",
            config_path: hermes_home.join("plugins/mosaico"),
            detected: available.contains(&crate::session::Harness::Hermes),
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
             Set HOME to the real user home; MOSAICO_HOME only controls mosaico daemon state."
        );
    };
    Ok(PathBuf::from(home))
}

fn grok_home_dir(grok_home: Option<String>, home: &std::path::Path) -> PathBuf {
    grok_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".grok"))
}

fn hermes_home_dir(hermes_home: Option<String>, home: &std::path::Path) -> PathBuf {
    hermes_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".hermes"))
}

pub(super) fn claude_detected() -> Result<bool> {
    Ok(crate::config::detect_available_harnesses()?.contains(&crate::session::Harness::ClaudeCode))
}

/// The hook signature we dedupe by: `mosaico harness hook <host> --type <type>`.
fn sig(host: &str, ty: &str) -> String {
    format!("mosaico harness hook {host} --type {ty}")
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
        ("SessionStart", mk("session-start", 5)),
        ("SessionEnd", mk("session-end", 5)),
        ("UserPromptSubmit", mk("user-prompt-submit", 5)),
        ("PostToolUse", mk("post-tool-use", 5)),
        ("Stop", mk("stop", 5)),
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
            mk("session-start", 5, Some("startup|resume")),
        ),
        ("UserPromptSubmit", mk("user-prompt-submit", 5, None)),
        ("PostToolUse", mk("post-tool-use", 5, None)),
        ("Stop", mk("stop", 5, None)),
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
        ("SessionStart", mk("session-start", 5)),
        ("SessionEnd", mk("session-end", 5)),
        ("UserPromptSubmit", mk("user-prompt-submit", 5)),
        ("PostToolUse", mk("post-tool-use", 5)),
        ("Stop", mk("stop", 5)),
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
            assert!(err.contains("MOSAICO_HOME only controls"));
        }
    }

    #[test]
    fn grok_home_honors_override_and_defaults_under_home() {
        let home = PathBuf::from("/Users/alice");
        assert_eq!(
            grok_home_dir(Some("/tmp/grok".to_string()), &home),
            PathBuf::from("/tmp/grok")
        );
        assert_eq!(grok_home_dir(None, &home), home.join(".grok"));
        assert_eq!(
            grok_home_dir(Some(String::new()), &home),
            home.join(".grok")
        );
    }

    #[test]
    fn hermes_home_honors_override_and_defaults_under_home() {
        let home = PathBuf::from("/Users/alice");
        assert_eq!(
            hermes_home_dir(Some("/tmp/hermes".to_string()), &home),
            PathBuf::from("/tmp/hermes")
        );
        assert_eq!(hermes_home_dir(None, &home), home.join(".hermes"));
    }
}
