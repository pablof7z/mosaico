use anyhow::{bail, Context, Result};
use std::io::IsTerminal;

pub(super) async fn resume(args: super::args::ResumeArgs) -> Result<()> {
    let workspace = args
        .workspace
        .map(absolute_workspace)
        .transpose()?
        .map(|path| path.to_string_lossy().into_owned());
    let response = super::daemon_call_async(
        "pty_resume_native",
        serde_json::json!({
            "native_id": args.harness_id,
            "workspace": workspace,
        }),
    )
    .await
    .with_context(|| format!("resuming native session {:?}", args.harness_id))?;
    let action = response["action"]
        .as_str()
        .context("pty_resume_native did not return an action")?;
    let handle = response["handle"]
        .as_str()
        .context("pty_resume_native did not return a handle")?;
    let harness = response["harness"]
        .as_str()
        .context("pty_resume_native did not return a harness")?;
    let pty_id = response["pty_id"]
        .as_str()
        .context("pty_resume_native did not return a PTY endpoint")?;
    match action {
        "attached" => eprintln!("Attached to @{handle} ({harness})"),
        "resumed" => eprintln!("Resumed @{handle} ({harness})"),
        "adopted" => eprintln!("Adopted and resumed @{handle} ({harness})"),
        other => bail!("pty_resume_native returned unknown action {other:?}"),
    }
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        crate::pty::attach(pty_id, handle)?;
    } else {
        eprintln!("PTY endpoint: {pty_id}");
    }
    Ok(())
}

fn absolute_workspace(path: std::path::PathBuf) -> Result<std::path::PathBuf> {
    let path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    path.canonicalize()
        .with_context(|| format!("resolving workspace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_override_becomes_absolute() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(
            absolute_workspace(temp.path().to_path_buf()).unwrap(),
            temp.path().canonicalize().unwrap()
        );
    }
}
