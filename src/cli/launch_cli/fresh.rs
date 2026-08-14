use std::io::IsTerminal;

use anyhow::{bail, Context as _, Result};

pub(super) struct FreshLaunchRequest {
    pub(super) agent: String,
    pub(super) root: String,
    pub(super) channel: Option<String>,
    pub(super) session_name: Option<String>,
    pub(super) prompt: Option<String>,
    pub(super) extra_args: Vec<String>,
}

pub(super) async fn launch(request: FreshLaunchRequest) -> Result<()> {
    let agent = request.agent.clone();
    let params = pty_spawn_params(&request, &std::env::current_dir().unwrap_or_default());
    let spawned = super::super::daemon_call_async("pty_spawn", params)
        .await
        .with_context(|| format!("launch of agent {agent:?} failed"))?;
    let transport = spawned["transport"]
        .as_str()
        .context("pty_spawn did not return transport")?;
    match crate::session_host::transport::TransportKind::parse(transport) {
        Some(crate::session_host::transport::TransportKind::Pty) => attach_pty(&spawned),
        Some(
            crate::session_host::transport::TransportKind::Acp
            | crate::session_host::transport::TransportKind::AppServer
            | crate::session_host::transport::TransportKind::PiRpc,
        ) => report_headless(&spawned),
        None => bail!("pty_spawn returned unknown transport {transport:?}"),
    }
}

fn attach_pty(spawned: &serde_json::Value) -> Result<()> {
    let socket = spawned["pty_socket"]
        .as_str()
        .context("pty_spawn did not return pty_socket")?;
    let handle = spawned["handle"]
        .as_str()
        .context("pty_spawn did not return the agent handle")?;
    eprintln!("Launched {handle}");
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        crate::pty::attach(socket, handle)?;
    }
    Ok(())
}

fn report_headless(spawned: &serde_json::Value) -> Result<()> {
    let session = spawned["pty_id"]
        .as_str()
        .context("pty_spawn did not return pty_id")?;
    eprintln!("[mosaico hosted] session: {session}");
    eprintln!(
        "[mosaico hosted] headless agent launched; it responds to channel mentions (no PTY to attach)"
    );
    Ok(())
}

fn pty_spawn_params(request: &FreshLaunchRequest, cwd: &std::path::Path) -> serde_json::Value {
    serde_json::json!({
        "agent": request.agent,
        "root": request.root,
        "cwd": cwd,
        "channel": request.channel,
        "session_name": request.session_name,
        "prompt": request.prompt,
        "extra_args": request.extra_args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_defers_agent_bundle_resolution_to_daemon() {
        let params = pty_spawn_params(
            &FreshLaunchRequest {
                agent: "codex".into(),
                root: "mosaico".into(),
                channel: Some("design".into()),
                session_name: Some("forensic".into()),
                prompt: Some("start here".into()),
                extra_args: vec!["--yolo".into()],
            },
            std::path::Path::new("/tmp/project"),
        );
        assert_eq!(
            params,
            serde_json::json!({
                "agent": "codex",
                "root": "mosaico",
                "cwd": "/tmp/project",
                "channel": "design",
                "session_name": "forensic",
                "prompt": "start here",
                "extra_args": ["--yolo"],
            })
        );
    }

    #[test]
    fn unscoped_launch_keeps_cwd_without_inventing_a_channel() {
        let params = pty_spawn_params(
            &FreshLaunchRequest {
                agent: "codex".into(),
                root: String::new(),
                channel: None,
                session_name: None,
                prompt: None,
                extra_args: Vec::new(),
            },
            std::path::Path::new("/tmp/work"),
        );

        assert_eq!(params["root"], "");
        assert_eq!(params["cwd"], "/tmp/work");
        assert!(params["channel"].is_null());
    }

    #[test]
    fn daemon_transport_contract_keeps_app_server_headless() {
        use crate::session_host::transport::TransportKind;

        assert_eq!(TransportKind::parse("pty"), Some(TransportKind::Pty));
        assert_eq!(TransportKind::parse("acp"), Some(TransportKind::Acp));
        assert_eq!(
            TransportKind::parse("app-server"),
            Some(TransportKind::AppServer)
        );
        assert_eq!(TransportKind::parse("app_server"), None);
    }
}
