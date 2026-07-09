use super::*;

pub(crate) async fn bootstrap_pty_session_start(
    state: &Arc<DaemonState>,
    meta: &crate::pty::LaunchMetadata,
    channel: Option<&str>,
    resume_id: Option<&str>,
) -> Result<String> {
    let harness = infer_harness(&meta.command);
    let watch_pid = i32::try_from(meta.supervisor_pid).ok();
    let response = rpc_session_start(
        state,
        &serde_json::json!({
            "agent": &meta.agent,
            "harness": harness.as_str(),
            "cwd": &meta.cwd,
            "channel": channel,
            "watch_pid": watch_pid,
            "pty_session": &meta.id,
            "pty_socket": &meta.socket,
            "resume_id": resume_id,
        }),
        None,
    )
    .await?;
    response["session_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("session_start bootstrap returned no session_id"))
}

pub(crate) async fn bootstrap_exec_session_start(
    state: &Arc<DaemonState>,
    agent: &str,
    harness: Harness,
    cwd: &str,
    channel: Option<&str>,
    watch_pid: i32,
    native_id: Option<&str>,
) -> Result<String> {
    let response = rpc_session_start(
        state,
        &serde_json::json!({
            "agent": agent,
            "harness": harness.as_str(),
            "cwd": cwd,
            "channel": channel,
            "watch_pid": watch_pid,
            "session_id": native_id,
        }),
        None,
    )
    .await?;
    response["session_id"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("exec session_start bootstrap returned no session_id"))
}

fn infer_harness(command: &[String]) -> Harness {
    match command_binary(command).as_deref() {
        Some("claude" | "claude-code") => Harness::ClaudeCode,
        Some("codex") => Harness::Codex,
        Some("opencode") => Harness::Opencode,
        Some("grok") => Harness::Grok,
        _ => Harness::Unknown,
    }
}

fn command_binary(command: &[String]) -> Option<String> {
    let mut index = 0;
    if command.first().is_some_and(|arg| base_name(arg) == "env") {
        index = 1;
        while let Some(arg) = command.get(index) {
            if arg == "-u" || arg == "--unset" {
                index += 2;
                continue;
            }
            if arg.starts_with("-u") && arg.len() > 2 {
                index += 1;
                continue;
            }
            if arg == "-i" || arg == "--ignore-environment" {
                index += 1;
                continue;
            }
            if arg.contains('=') {
                index += 1;
                continue;
            }
            break;
        }
    }
    command.get(index).map(|arg| base_name(arg))
}

fn base_name(arg: &str) -> String {
    std::path::Path::new(arg)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(arg)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_wrapped_codex_command() {
        let command = vec![
            "env".to_string(),
            "-u".to_string(),
            "CLAUDE_CODE_SESSION_ID".to_string(),
            "TENEX_EDGE_ORDINAL=1".to_string(),
            "/usr/local/bin/codex".to_string(),
            "--yolo".to_string(),
        ];

        assert_eq!(infer_harness(&command), Harness::Codex);
    }
}
