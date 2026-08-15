use super::*;

mod context_contracts;
mod harness_inventory;

fn harness(id: &'static str, path: std::path::PathBuf) -> Harness {
    Harness {
        id,
        display: id,
        config_path: path,
        detected: true,
    }
}

fn opts(all: bool, harness: Option<&str>) -> InstallOpts {
    InstallOpts {
        all,
        harness: harness.map(str::to_string),
        ..InstallOpts::default()
    }
}

#[test]
fn native_pre_tool_hook_is_installed_only_for_confirmed_json_hosts() {
    let claude = harness("claude-code", "claude.json".into());
    let codex = harness("codex", "codex.json".into());
    let grok = harness("grok", "grok.json".into());
    let goose = harness("goose", "goose".into());

    for host in [&claude, &codex] {
        let entries = config::hook_entries(host);
        let pre_tool = entries
            .iter()
            .find(|(event, _)| *event == "PreToolUse")
            .unwrap();
        assert_eq!(
            pre_tool
                .1
                .pointer("/hooks/0/command")
                .and_then(|v| v.as_str()),
            Some(if host.id == "codex" {
                "mosaico harness hook codex --type pre-tool-use"
            } else {
                "mosaico harness hook claude-code --type pre-tool-use"
            })
        );
        assert!(pre_tool.1.get("matcher").is_some());
    }
    assert!(config::hook_entries(&grok)
        .iter()
        .all(|(event, _)| *event != "PreToolUse"));
    assert!(config::hook_entries(&goose).is_empty());
}

#[test]
fn hermes_pre_tool_bridge_injects_warnings_and_returns_native_blocks() {
    let module =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("integrations/hermes/__init__.py");
    let script = r#"
import importlib.util, sys
spec = importlib.util.spec_from_file_location("mosaico_hermes", sys.argv[1])
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class Context:
    def __init__(self):
        self.hooks = {}
        self.messages = []
    def register_hook(self, name, callback):
        self.hooks[name] = callback
    def inject_message(self, message):
        self.messages.append(message)
        return True

ctx = Context()
module.register(ctx)
assert "pre_tool_call" in ctx.hooks
module._invoke = lambda *_: {"decision": "warn", "message": "WARN: /beta"}
assert ctx.hooks["pre_tool_call"](tool_name="read_file", args={"path": "/beta/x"}) is None
assert ctx.messages == ["WARN: /beta"]
module._invoke = lambda *_: {"decision": "deny", "message": "DENIED: /beta"}
assert ctx.hooks["pre_tool_call"](tool_name="write_file", args={"path": "/beta/x"}) == {
    "action": "block", "message": "DENIED: /beta"
}
"#;
    // Installer tests intentionally mutate PATH in parallel. Use the system
    // interpreter directly so this contract test remains isolated from them.
    let output = std::process::Command::new("/usr/bin/python3")
        .args(["-c", script])
        .arg(module)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn merge_hooks_preserves_foreign_groups_and_replaces_ours() {
    let mut root = serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "pc hook inject --harness codex",
                        "timeout": 30
                    }]
                },
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type old",
                        "timeout": 1
                    }]
                }
            ]
        }
    });

    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);

    let groups = root
        .pointer("/hooks/UserPromptSubmit")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().any(|g| {
        g.pointer("/hooks/0/command")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "pc hook inject --harness codex")
    }));
    assert!(groups.iter().any(|g| {
        g.pointer("/hooks/0/command")
            .and_then(|v| v.as_str())
            .is_some_and(|c| c == "mosaico harness hook codex --type user-prompt-submit")
    }));
}

#[test]
fn uninstall_removes_ours_and_empty_events_only() {
    let mut root = serde_json::json!({
        "hooks": {
            "Stop": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type stop",
                        "timeout": 30
                    }]
                }
            ],
            "UserPromptSubmit": [
                {
                    "hooks": [{
                        "type": "command",
                        "command": "pc hook inject --harness codex",
                        "timeout": 30
                    }]
                },
                {
                    "hooks": [{
                        "type": "command",
                        "command": "mosaico harness hook codex --type user-prompt-submit",
                        "timeout": 30
                    }]
                }
            ]
        }
    });

    let removed = merge_hooks(&mut root, &config::codex_hook_entries(), "codex", true);

    assert_eq!(removed, 2);
    assert!(root.pointer("/hooks/Stop").is_none());
    let groups = root
        .pointer("/hooks/UserPromptSubmit")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0]
            .pointer("/hooks/0/command")
            .and_then(|v| v.as_str()),
        Some("pc hook inject --harness codex")
    );
}

#[test]
fn write_json_creates_parent_directories() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("a/b/hooks.json");
    write_json(&path, &serde_json::json!({"hooks": {}})).unwrap();
    assert!(path.exists());
}

#[test]
fn status_detects_installed_codex_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("codex", temp.path().join("hooks.json"));
    let mut root = serde_json::json!({});
    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);
    write_json(&h.config_path, &root).unwrap();

    assert!(is_installed(&h));
}

#[test]
fn removed_codex_root_hook_shape_is_not_recognized() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("codex", temp.path().join("hooks.json"));
    write_json(
        &h.config_path,
        &serde_json::json!({
            "SessionStart": [config::codex_hook_entries()[0].1.clone()],
            "UserPromptSubmit": [config::codex_hook_entries()[1].1.clone()],
        }),
    )
    .unwrap();

    assert!(!is_installed(&h));
}

#[test]
fn installation_requires_at_least_one_wired_harness() {
    let temp = tempfile::tempdir().unwrap();
    let codex = harness("codex", temp.path().join("hooks.json"));
    let opencode = harness("opencode", temp.path().join("mosaico.ts"));

    assert!(![&codex, &opencode].into_iter().any(is_installed));

    let mut root = serde_json::json!({});
    merge_hooks(&mut root, &config::codex_hook_entries(), "codex", false);
    write_json(&codex.config_path, &root).unwrap();

    assert!([&codex, &opencode].into_iter().any(is_installed));
}

#[test]
fn pi_installation_requires_the_current_owned_extension() {
    let temp = tempfile::tempdir().unwrap();
    let h = harness("pi", temp.path().join("mosaico.ts"));
    write_text(&h.config_path, "export default function stale() {}\n").unwrap();
    assert!(!is_installed(&h));

    install_pi(&h, &InstallOpts::default(), false).unwrap();
    assert!(is_installed(&h));
    assert_eq!(
        std::fs::read_to_string(&h.config_path).unwrap(),
        PI_EXTENSION_TS
    );
    assert_eq!(
        std::fs::read_to_string(h.config_path.with_file_name("protocol.ts")).unwrap(),
        PI_PROTOCOL_TS
    );
    assert_eq!(
        std::fs::read_to_string(h.config_path.with_file_name("status.ts")).unwrap(),
        PI_STATUS_TS
    );
    assert_eq!(
        std::fs::read_to_string(h.config_path.with_file_name("tools.ts")).unwrap(),
        PI_TOOLS_TS
    );
}
