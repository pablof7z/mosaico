use super::*;

pub(crate) struct PtySessionStart<'a> {
    pub(crate) channel: Option<&'a str>,
    pub(crate) channels: &'a [String],
    pub(crate) resume_id: Option<&'a str>,
    pub(crate) dispatch_event: Option<&'a str>,
    pub(crate) session_name: Option<&'a str>,
    pub(crate) durable_reservation: Option<&'a str>,
}

pub(crate) async fn bootstrap_pty_session_start(
    state: &Arc<DaemonState>,
    meta: &crate::pty::LaunchMetadata,
    request: PtySessionStart<'_>,
) -> Result<String> {
    let harness = infer_harness(&meta.command);
    let watch_pid = i32::try_from(meta.supervisor_pid).ok();
    let response = rpc_session_start(
        state,
        &serde_json::json!({
            "agent": &meta.agent,
            "harness": harness.as_str(),
            "cwd": &meta.cwd,
            "channel": request.channel,
            "channels": request.channels,
            "watch_pid": watch_pid,
            "pty_session": &meta.id,
            "resume_id": request.resume_id,
            "dispatch_event": request.dispatch_event,
            "session_name": request.session_name,
            "durable_reservation": request.durable_reservation,
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
    // The ACP path records the real argv (defect #8). The claude-agent-acp adapter
    // launches through `npx --yes @agentclientprotocol/claude-agent-acp`, so
    // argv[0] is `npx`, not
    // `claude` — recognize the adapter package to keep the harness correct.
    if command.iter().any(|a| a.contains("claude-agent-acp")) {
        return Harness::ClaudeCode;
    }
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
