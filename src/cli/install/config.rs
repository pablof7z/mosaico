//! Harness discovery and hook entry configuration.

use anyhow::{bail, Result};
use std::path::PathBuf;

pub const OPENCODE_PLUGIN_TS: &str = include_str!("../../../integrations/opencode/mosaico.ts");
pub const PI_EXTENSION_TS: &str = include_str!("../../../integrations/pi/mosaico.ts");
pub const PI_DELIVERY_TS: &str = include_str!("../../../integrations/pi/delivery.ts");
pub const PI_PROTOCOL_TS: &str = include_str!("../../../integrations/pi/protocol.ts");
pub const PI_STATUS_TS: &str = include_str!("../../../integrations/pi/status.ts");
pub const PI_TOOLS_TS: &str = include_str!("../../../integrations/pi/tools.ts");
pub const PI_EXTENSION_FILES: &[(&str, &str)] = &[
    ("index.ts", PI_EXTENSION_TS),
    ("delivery.ts", PI_DELIVERY_TS),
    ("protocol.ts", PI_PROTOCOL_TS),
    ("status.ts", PI_STATUS_TS),
    ("tools.ts", PI_TOOLS_TS),
];
pub const HERMES_PLUGIN_YAML: &str = include_str!("../../../integrations/hermes/plugin.yaml");
pub const HERMES_PLUGIN_PY: &str = include_str!("../../../integrations/hermes/__init__.py");

#[derive(Debug)]
pub struct Harness {
    pub id: &'static str,
    pub display: &'static str,
    pub config_path: PathBuf,
    pub detected: bool,
}

impl Harness {
    /// Native executable and default Mosaico launch target for this harness.
    pub fn command(&self) -> &'static str {
        match self.id {
            "claude-code" => "claude",
            "codex" => "codex",
            "opencode" => "opencode",
            "grok" => "grok",
            "goose" => "goose",
            "hermes" => "hermes",
            "kimi" => "kimi",
            "pi" => "pi",
            _ => unreachable!("unknown installer harness {}", self.id),
        }
    }
}

pub fn harnesses() -> Result<Vec<Harness>> {
    let home = home_dir()?;
    let grok_home = grok_home_dir(std::env::var("GROK_HOME").ok(), &home);
    let hermes_home = hermes_home_dir(std::env::var("HERMES_HOME").ok(), &home);
    let kimi_home = kimi_home_dir(std::env::var("KIMI_CODE_HOME").ok(), &home);
    let pi_agent_dir = pi_agent_dir(std::env::var("PI_CODING_AGENT_DIR").ok(), &home);
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
        Harness {
            id: "kimi",
            display: "Kimi Code",
            config_path: kimi_home.join("config.toml"),
            detected: available.contains(&crate::session::Harness::Kimi),
        },
        Harness {
            id: "pi",
            display: "Pi",
            config_path: pi_agent_dir.join("extensions/mosaico"),
            detected: available.contains(&crate::session::Harness::Pi),
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
             Set HOME to the real user home; MOSAICO and MOSAICO_HOME only select mosaico daemon state."
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

fn kimi_home_dir(kimi_home: Option<String>, home: &std::path::Path) -> PathBuf {
    kimi_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".kimi-code"))
}

fn pi_agent_dir(pi_agent_dir: Option<String>, home: &std::path::Path) -> PathBuf {
    pi_agent_dir
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".pi/agent"))
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
        (
            "PreToolUse",
            with_matcher(
                mk("pre-tool-use", 5),
                "Read|Write|Edit|MultiEdit|NotebookEdit|Glob|Grep",
            ),
        ),
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
        (
            "PreToolUse",
            mk(
                "pre-tool-use",
                5,
                Some("Read|Write|Edit|MultiEdit|NotebookEdit|Glob|Grep|view_image"),
            ),
        ),
        ("PostToolUse", mk("post-tool-use", 5, None)),
        ("Stop", mk("stop", 5, None)),
    ]
}

fn with_matcher(mut entry: serde_json::Value, matcher: &str) -> serde_json::Value {
    entry["matcher"] = matcher.into();
    entry
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
#[path = "config/tests.rs"]
mod tests;
