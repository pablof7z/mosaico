use anyhow::{bail, Context, Result};
use std::io::IsTerminal;

pub(super) async fn launch_if_known(request: &super::args::LaunchRequest) -> Result<bool> {
    if !can_select_existing_session(request) {
        return Ok(false);
    }
    attach_or_resume(&request.agent, &request.extra_args).await
}

pub(in crate::cli) async fn attach_or_resume(
    selector: &str,
    extra_args: &[String],
) -> Result<bool> {
    let response = crate::cli::daemon_call_async(
        "pty_launch_existing",
        serde_json::json!({
            "session": selector,
            "extra_args": extra_args,
        }),
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
    if action == "already-running" {
        bail!(
            "{handle} is already running without an attachable PTY; open `mosaico` for explicit takeover"
        );
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

fn can_select_existing_session(request: &super::args::LaunchRequest) -> bool {
    request.channel.is_none() && request.session_name.is_none() && request.prompt.is_none()
}

#[cfg(test)]
mod tests {
    use super::can_select_existing_session;
    use crate::cli::launch_cli::args::LaunchRequest;

    fn request() -> LaunchRequest {
        LaunchRequest {
            agent: "echo-codex".into(),
            channel: None,
            session_name: None,
            prompt: None,
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn launch_arguments_do_not_bypass_existing_session_resolution() {
        let mut plain = request();
        assert!(can_select_existing_session(&plain));
        plain.extra_args.push("--yolo".into());
        assert!(can_select_existing_session(&plain));
        plain.prompt = Some("fresh prompt".into());
        assert!(!can_select_existing_session(&plain));
        let mut named = request();
        named.session_name = Some("hi".into());
        assert!(!can_select_existing_session(&named));
    }
}
