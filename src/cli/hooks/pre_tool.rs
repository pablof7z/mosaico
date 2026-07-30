use super::HostDef;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
}

impl Access {
    fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

pub(super) async fn check(
    host: &HostDef,
    session_id: &str,
    cwd: &Path,
    raw: &Value,
) -> Option<String> {
    if !matches!(host.name, "claude-code" | "codex" | "opencode" | "hermes") {
        return None;
    }
    let (access, path) = direct_path(raw)?;
    let params = crate::cli::rpc_params(serde_json::json!({
        "harness_session": session_id,
        "harness": host.name,
        "cwd": cwd.to_string_lossy(),
        "access": access.as_str(),
        "path": path,
    }));
    let result = super::super::daemon_call_hook_async("cross_project_path_classify", params)
        .await
        .ok()?;
    render(host.name, &result)
}

fn direct_path(raw: &Value) -> Option<(Access, &str)> {
    let tool = raw.get("tool_name")?.as_str()?.to_ascii_lowercase();
    let input = raw.get("tool_input")?.as_object()?;
    let access = match tool.as_str() {
        "read" | "glob" | "grep" | "view_image" | "read_file" => Access::Read,
        "write" | "edit" | "multiedit" | "notebookedit" | "write_file" => Access::Write,
        // Bash, Shell, terminal, apply_patch, patch, and unknown/plugin tools
        // are intentionally outside this cooperative direct-path heuristic.
        _ => return None,
    };
    [
        "file_path",
        "filePath",
        "path",
        "notebook_path",
        "notebookPath",
    ]
    .into_iter()
    .find_map(|key| input.get(key).and_then(Value::as_str))
    .filter(|path| !path.is_empty())
    .map(|path| (access, path))
}

fn render(host: &str, result: &Value) -> Option<String> {
    let decision = result.get("decision")?.as_str()?;
    let message = result.get("message")?.as_str()?;
    match host {
        "claude-code" | "codex" if decision == "warn" => Some(
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "additionalContext": message,
                }
            })
            .to_string(),
        ),
        "claude-code" | "codex" if decision == "deny" => Some(
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": message,
                }
            })
            .to_string(),
        ),
        "opencode" | "hermes" if matches!(decision, "warn" | "deny") => Some(
            serde_json::json!({
                "decision": decision,
                "message": message,
            })
            .to_string(),
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(tool: &str, input: Value) -> Value {
        serde_json::json!({"tool_name": tool, "tool_input": input})
    }

    #[test]
    fn recognizes_only_direct_paths_on_known_file_tools() {
        assert_eq!(
            direct_path(&payload(
                "Read",
                serde_json::json!({"file_path":"README.md"})
            )),
            Some((Access::Read, "README.md"))
        );
        assert_eq!(
            direct_path(&payload(
                "NotebookEdit",
                serde_json::json!({"notebook_path":"notes.ipynb"})
            )),
            Some((Access::Write, "notes.ipynb"))
        );
        assert_eq!(
            direct_path(&payload(
                "write",
                serde_json::json!({"filePath":"src/lib.rs"})
            )),
            Some((Access::Write, "src/lib.rs"))
        );
    }

    #[test]
    fn ignores_shell_patch_indirect_and_unknown_tools() {
        for event in [
            payload("Bash", serde_json::json!({"command":"cat ../other/file"})),
            payload("terminal", serde_json::json!({"path":"../other/file"})),
            payload(
                "apply_patch",
                serde_json::json!({"patch":"*** Update File: ../other/file"}),
            ),
            payload("plugin_tool", serde_json::json!({"path":"../other/file"})),
            payload("Read", serde_json::json!({"query":"../other/file"})),
        ] {
            assert_eq!(direct_path(&event), None);
        }
    }

    #[test]
    fn claude_and_codex_warning_and_deny_contracts_are_native() {
        for host in ["claude-code", "codex"] {
            let warning: Value = serde_json::from_str(
                &render(
                    host,
                    &serde_json::json!({"decision":"warn","message":"WARN: other workspace"}),
                )
                .unwrap(),
            )
            .unwrap();
            let denial: Value = serde_json::from_str(
                &render(
                    host,
                    &serde_json::json!({"decision":"deny","message":"DENIED: other workspace"}),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                warning["hookSpecificOutput"]["additionalContext"],
                "WARN: other workspace"
            );
            assert!(warning["hookSpecificOutput"]
                .get("permissionDecision")
                .is_none());
            assert_eq!(denial["hookSpecificOutput"]["permissionDecision"], "deny");
        }
    }

    #[test]
    fn plugin_hosts_receive_a_small_decision_object() {
        for host in ["opencode", "hermes"] {
            let rendered: Value = serde_json::from_str(
                &render(
                    host,
                    &serde_json::json!({"decision":"deny","message":"stay in workspace"}),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                rendered,
                serde_json::json!({"decision":"deny","message":"stay in workspace"})
            );
        }
        assert_eq!(
            render(
                "grok",
                &serde_json::json!({"decision":"deny","message":"ignored"})
            ),
            None
        );
    }
}
