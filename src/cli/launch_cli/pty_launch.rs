use std::io::IsTerminal;

use anyhow::{Context as _, Result};

pub(super) enum CommandSource {
    Command(Vec<String>),
    Bundle(String),
}

pub(super) struct PtyLaunchRequest {
    pub(super) agent: String,
    pub(super) root: String,
    pub(super) channel: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) source: CommandSource,
    pub(super) extra_args: Vec<String>,
    pub(super) prompt: Option<String>,
}

pub(super) async fn launch(request: PtyLaunchRequest) -> Result<()> {
    let agent = request.agent.clone();
    let params = pty_spawn_params(&request, &std::env::current_dir().unwrap_or_default());
    let spawned = super::super::daemon_call_async("pty_spawn", params)
        .await
        .with_context(|| format!("interactive PTY launch of agent {agent:?} failed"))?;
    let socket = spawned["pty_socket"]
        .as_str()
        .context("pty_spawn did not return pty_socket")?;
    let handle = spawned["handle"]
        .as_str()
        .context("pty_spawn did not return the agent handle")?;

    eprintln!("Launched {handle}");
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(());
    }
    crate::pty::attach(socket, handle)
}

fn pty_spawn_params(request: &PtyLaunchRequest, cwd: &std::path::Path) -> serde_json::Value {
    let (launch, args) = match &request.source {
        CommandSource::Command(base) => {
            let args = super::launch_command::extra_args_without_duplicate_suffix(
                base,
                request.extra_args.clone(),
            );
            (
                serde_json::json!({ "kind": "pty-command", "argv": base }),
                args,
            )
        }
        CommandSource::Bundle(bundle) => (
            serde_json::json!({ "kind": "pty-bundle", "bundle": bundle }),
            request.extra_args.clone(),
        ),
    };
    serde_json::json!({
        "agent": request.agent,
        "root": request.root,
        "launch": launch,
        "args": args,
        "cwd": cwd,
        "channel": request.channel,
        "session_name": request.session_name,
        "prompt": request.prompt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_source_sends_selected_argv_and_deduplicated_args_to_daemon() {
        let params = pty_spawn_params(
            &PtyLaunchRequest {
                agent: "codex".into(),
                root: "mosaico".into(),
                channel: Some("design".into()),
                session_name: Some("forensic".into()),
                source: CommandSource::Command(vec!["codex".into(), "--yolo".into()]),
                extra_args: vec!["--yolo".into()],
                prompt: Some("start here".into()),
            },
            std::path::Path::new("/tmp/project"),
        );
        assert_eq!(
            params,
            serde_json::json!({
                "agent": "codex",
                "root": "mosaico",
                "launch": { "kind": "pty-command", "argv": ["codex", "--yolo"] },
                "args": [],
                "cwd": "/tmp/project",
                "channel": "design",
                "session_name": "forensic",
                "prompt": "start here",
            })
        );
    }

    #[test]
    fn bundle_source_defers_resolution_and_profile_materialization_to_daemon() {
        let params = pty_spawn_params(
            &PtyLaunchRequest {
                agent: "reviewer".into(),
                root: "mosaico".into(),
                channel: None,
                session_name: None,
                source: CommandSource::Bundle("codex-yolo".into()),
                extra_args: vec!["--search".into()],
                prompt: None,
            },
            std::path::Path::new("/tmp/project"),
        );
        assert_eq!(
            params["launch"],
            serde_json::json!({ "kind": "pty-bundle", "bundle": "codex-yolo" })
        );
        assert_eq!(params["args"], serde_json::json!(["--search"]));
    }
}
