use crate::daemon::server::DaemonState;
use crate::session_host::registry::{
    apply_agent_def_args, build_resume_command, find_spawn_def, resolve_spawn_entry,
    resume_shape_for_bin,
};
use anyhow::{Context, Result};
use std::sync::Arc;

pub(super) fn workspace_abs_path(
    state: &Arc<DaemonState>,
    channel: &str,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    if let Some(cwd) = client_cwd {
        let abs = cwd.to_string_lossy().to_string();
        let now = crate::util::now_secs();
        // The recorded workspace path is what the resume path reads back; if the
        // write is dropped, a later resume falls into the "no workspace" branch and
        // we'd spawn in the wrong directory. Propagate the failure, don't swallow.
        state
            .with_store(|s| s.upsert_workspace(channel, &abs, now))
            .with_context(|| format!("recording workspace path for {channel:?}"))?;
        return Ok(abs);
    }
    // Resume path (no client cwd): the workspace path MUST already be recorded.
    // Never guess the daemon's current_dir here; an unrelated daemon cwd would
    // land the agent in the wrong directory. Fail loud on a read error or
    // missing row.
    let abs = state
        .with_store(|s| s.workspace_path(channel))
        .with_context(|| format!("looking up workspace path for {channel:?}"))?;
    abs.ok_or_else(|| {
        anyhow::anyhow!("cannot resolve workspace path for {channel:?} (no recorded path)")
    })
}

async fn open_agent_session(
    slug: &str,
    root: &str,
    abs_path: &str,
    command: &[String],
    group: Option<&str>,
    ephemeral: bool,
) -> Result<crate::pty::LaunchMetadata> {
    let meta = crate::pty::spawn_session(crate::pty::SpawnSessionArgs {
        id: None,
        agent: slug.to_string(),
        root: root.to_string(),
        cwd: std::path::PathBuf::from(abs_path),
        channel: group.filter(|g| !g.is_empty()).map(str::to_string),
        ephemeral,
        command: command.to_vec(),
    })?;
    Ok(meta)
}

/// Spawn a new PTY-hosted harness in `root`'s directory. Returns the
/// supervisor session id.
pub async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    spawn_agent_inner(
        state,
        slug,
        root,
        launch_args,
        base_override,
        group,
        client_cwd,
        false,
    )
    .await
}

pub async fn spawn_ephemeral_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    spawn_agent_inner(
        state,
        slug,
        root,
        launch_args,
        base_override,
        group,
        client_cwd,
        true,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn spawn_agent_inner(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    ephemeral: bool,
) -> Result<String> {
    let (base_command, agent_def) = match base_override {
        Some(cmd) => (cmd, None),
        None => resolve_spawn_entry(slug)?,
    };
    let mut agent_command = apply_agent_def_args(base_command, slug, agent_def);
    if !launch_args.is_empty() {
        agent_command.extend(launch_args);
    }
    let _ = find_spawn_def(slug);

    let abs_path = workspace_abs_path(state, root, client_cwd)?;
    let meta = open_agent_session(slug, root, &abs_path, &agent_command, group, ephemeral).await?;
    let pty_id = meta.id.clone();
    if let Err(e) =
        crate::daemon::server::session_start::bootstrap_pty_session_start(state, &meta, group, None)
            .await
    {
        let _ = crate::pty::kill(&pty_id);
        return Err(e.context("registering PTY-hosted session"));
    }
    Ok(pty_id)
}

/// Resume a prior session by replaying its harness with the native resume token.
pub async fn resume_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    resume_id: &str,
) -> Result<String> {
    resume_agent_in_channel(state, slug, root, root, resume_id).await
}

/// Resume a prior session into an explicit channel while using `root` to
/// resolve the working directory.
pub async fn resume_agent_in_channel(
    state: &Arc<DaemonState>,
    slug: &str,
    root: &str,
    group: &str,
    resume_id: &str,
) -> Result<String> {
    if resume_id.is_empty() {
        anyhow::bail!("session has no resume token (not resumable)");
    }

    let (base, _agent_def) = resolve_spawn_entry(slug)?;
    let bin = base.first().map(String::as_str).unwrap_or("");
    let shape = resume_shape_for_bin(bin).with_context(|| {
        format!("don't know how to resume harness binary {bin:?} (agent {slug:?})")
    })?;
    let resume_command = build_resume_command(&base, shape, resume_id);

    let abs_path = workspace_abs_path(state, root, None)?;
    // A resumed claude/codex session re-registers under the SAME session_id, so it
    // deterministically re-derives its own pubkey — no explicit hint needed.
    let meta =
        open_agent_session(slug, root, &abs_path, &resume_command, Some(group), false).await?;
    let pty_id = meta.id.clone();
    if let Err(e) = crate::daemon::server::session_start::bootstrap_pty_session_start(
        state,
        &meta,
        Some(group),
        Some(resume_id),
    )
    .await
    {
        let _ = crate::pty::kill(&pty_id);
        return Err(e.context("registering resumed PTY-hosted session"));
    }
    Ok(pty_id)
}
