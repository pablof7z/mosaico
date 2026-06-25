//! Harness discovery and hook entry configuration.

use std::path::PathBuf;

pub const OPENCODE_PLUGIN_TS: &str = include_str!("../../../integrations/opencode/tenex-edge.ts");

#[derive(Debug)]
pub struct Harness {
    pub id: &'static str,
    pub display: &'static str,
    pub config_path: PathBuf,
    pub detected: bool,
}

pub fn harnesses() -> Vec<Harness> {
    let home = home_dir();
    vec![
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
    ]
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn bin_on_path(bin: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(bin).is_file())
}

/// The hook signature we dedupe by: `tenex-edge hook --host <host> --type <type>`.
pub fn sig(host: &str, ty: &str) -> String {
    format!("tenex-edge hook --host {host} --type {ty}")
}

pub fn claude_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
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
        ("SessionStart", mk("session-start", 10)),
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
            mk("session-start", 30, Some("startup|resume")),
        ),
        ("UserPromptSubmit", mk("user-prompt-submit", 30, None)),
        ("PostToolUse", mk("post-tool-use", 10, None)),
        ("Stop", mk("stop", 30, None)),
    ]
}

pub fn grok_hook_entries() -> Vec<(&'static str, serde_json::Value)> {
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
        ("SessionStart", mk("session-start", 10)),
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
