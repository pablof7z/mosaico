use anyhow::{bail, Context, Result};
use std::io::IsTerminal;

pub(super) async fn launch_if_known(request: &super::args::LaunchRequest) -> Result<bool> {
    if !is_plain_session_selector(request) {
        return Ok(false);
    }
    attach_or_resume(&request.agent).await
}

pub(super) async fn attach_or_resume(selector: &str) -> Result<bool> {
    let response = crate::cli::daemon_call_async(
        "pty_launch_existing",
        serde_json::json!({ "session": selector }),
    )
    .await?;
    let action = response["action"]
        .as_str()
        .context("pty_launch_existing did not return an action")?;
    if action == "not-found" {
        return Ok(false);
    }
    let handle = response["handle"]
        .as_str()
        .context("pty_launch_existing did not return the agent handle")?;
    if action == "not-resumable" {
        bail!("{handle} has no live terminal and its harness cannot be resumed");
    }
    let pty_id = response["pty_id"]
        .as_str()
        .context("pty_launch_existing did not return pty_id")?;
    match action {
        "attached" => eprintln!("Attached to {handle}"),
        "resumed" => eprintln!("Resumed {handle}"),
        _ => bail!("pty_launch_existing returned unknown action {action:?}"),
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(true);
    }
    crate::pty::attach(pty_id, handle)?;
    Ok(true)
}

fn is_plain_session_selector(request: &super::args::LaunchRequest) -> bool {
    request.root.is_none()
        && request.channel.is_none()
        && request.session_name.is_none()
        && request.command_name.is_none()
        && request.harness.is_none()
        && !request.headless
        && request.override_command.is_empty()
        && request.extra_args.is_empty()
        && request.prompt.is_none()
}

#[cfg(test)]
mod tests {
    use super::is_plain_session_selector;
    use crate::cli::launch_cli::args::LaunchRequest;

    fn request() -> LaunchRequest {
        LaunchRequest {
            agent: "echo-codex".into(),
            root: None,
            channel: None,
            session_name: None,
            command_name: None,
            harness: None,
            headless: false,
            override_command: Vec::new(),
            extra_args: Vec::new(),
            prompt: None,
        }
    }

    #[test]
    fn only_a_bare_launch_reuses_an_existing_session() {
        let mut plain = request();
        assert!(is_plain_session_selector(&plain));
        plain.prompt = Some("fresh prompt".into());
        assert!(!is_plain_session_selector(&plain));
    }
}
