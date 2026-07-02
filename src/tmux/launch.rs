use crate::daemon::server::DaemonState;
use crate::tmux::pane::tmux_available;
use crate::tmux::registry::{
    apply_agent_def_args, build_resume_command, find_spawn_def, resolve_spawn_entry,
    resume_shape_for_bin,
};
use anyhow::{Context, Result};
use std::sync::Arc;

fn project_abs_path(
    state: &Arc<DaemonState>,
    project: &str,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    if let Some(cwd) = client_cwd {
        let abs = cwd.to_string_lossy().to_string();
        let now = crate::util::now_secs();
        // The recorded root is what the resume path reads back; if the write is
        // dropped, a later resume falls into the "no root" branch and we'd spawn
        // in the wrong directory. Propagate the failure instead of swallowing it.
        state
            .with_store(|s| s.upsert_project_root(project, &abs, now))
            .with_context(|| format!("recording project root for {project:?}"))?;
        return Ok(abs);
    }
    // Resume path (no client cwd): the project root MUST already be recorded.
    // Never guess the daemon's current_dir here — an unrelated cwd (or an empty
    // PathBuf on error) would spawn `tmux new-session -c ""` and land the agent
    // in the wrong/invalid directory. Fail loud on a read error or missing row.
    let root = state
        .with_store(|s| s.project_root(project))
        .with_context(|| format!("looking up project root for {project:?}"))?;
    root.ok_or_else(|| {
        anyhow::anyhow!("cannot resolve project root for {project:?} (no recorded path)")
    })
}

fn unique_session_name(slug: &str) -> String {
    let base = format!("te-{slug}");
    let existing: std::collections::HashSet<String> = match std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
    {
        // tmux exits non-zero with "no server running" when there are genuinely
        // no sessions — that is a legitimate empty list, not a failure.
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        // The binary couldn't be spawned at all: treating that as "no sessions"
        // risks colliding with a real session, so surface it loudly.
        Err(e) => {
            tracing::warn!(error = %e, "unique_session_name: failed to run `tmux list-sessions` — assuming no existing sessions (name collision possible)");
            std::collections::HashSet::new()
        }
    };
    if !existing.contains(&base) {
        return base;
    }
    for n in 2..10_000 {
        let name = format!("{base}-{n}");
        if !existing.contains(&name) {
            return name;
        }
    }
    format!("{base}-{}", std::process::id())
}

async fn open_agent_session(
    slug: &str,
    window_name: &str,
    abs_path: &str,
    command: &[String],
    group: Option<&str>,
    ordinal: Option<u32>,
) -> Result<String> {
    let session_name = unique_session_name(slug);
    let agent_env = format!("TENEX_EDGE_AGENT={slug}");
    let mut passthrough_env: Vec<String> = Vec::new();
    for key in ["TENEX_EDGE_HOME", "TENEX_CONFIG", "TENEX_EDGE_BIN"] {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                passthrough_env.push(format!("{key}={val}"));
            }
        }
    }
    if let Some(g) = group.filter(|g| !g.is_empty()) {
        passthrough_env.push(format!("TENEX_EDGE_CHANNEL={g}"));
    }
    // Exact ordinal for a mention-driven spawn of a specific `smithN` (issue #47):
    // the hook forwards TENEX_EDGE_ORDINAL so the daemon allocates that ordinal
    // rather than the lowest-free one.
    if let Some(ord) = ordinal {
        passthrough_env.push(format!("TENEX_EDGE_ORDINAL={ord}"));
    }

    let mut cmd_args: Vec<&str> = vec![
        "new-session",
        "-d",
        "-s",
        &session_name,
        "-n",
        window_name,
        "-c",
        abs_path,
        "-e",
        "TENEX_EDGE_SPAWNED=1",
        "-e",
        &agent_env,
    ];
    for e in &passthrough_env {
        cmd_args.push("-e");
        cmd_args.push(e.as_str());
    }
    cmd_args.extend_from_slice(&[
        "-PF",
        "#{pane_id}",
        "--",
        "env",
        "-u",
        "CLAUDE_CODE_SESSION_ID",
        "-u",
        "CLAUDE_CODE_CHILD_SESSION",
    ]);
    let cmd_strs: Vec<&str> = command.iter().map(|s| s.as_str()).collect();
    cmd_args.extend_from_slice(&cmd_strs);

    let out = tokio::process::Command::new("tmux")
        .args(&cmd_args)
        .output()
        .await
        .context("tmux new-session")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("tmux new-session failed: {stderr}");
    }

    let pane_id = String::from_utf8(out.stdout)
        .context("tmux new-session output")?
        .trim()
        .to_string();

    let tenex_bin = std::env::var("TENEX_EDGE_BIN")
        .ok()
        .filter(|s| !s.is_empty());
    let status_cmd_override = crate::config::Config::load()
        .ok()
        .and_then(|c| c.tmux_status_command);
    make_session_transparent(
        &session_name,
        tenex_bin.as_deref(),
        slug,
        abs_path,
        status_cmd_override.as_deref(),
    )?;

    Ok(pane_id)
}

fn make_session_transparent(
    session: &str,
    tenex_bin: Option<&str>,
    slug: &str,
    abs_path: &str,
    status_cmd_override: Option<&str>,
) -> Result<()> {
    let bin = tenex_bin.unwrap_or("tenex-edge");
    let statusline_cmd = status_cmd_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_statusline_cmd(bin));
    let options: Vec<(&str, String)> = vec![
        ("@te_agent", slug.to_string()),
        ("@te_cwd", abs_path.to_string()),
        ("status-style", "default".to_string()),
        ("status", "on".to_string()),
        ("status-interval", "3".to_string()),
        ("status-left", String::new()),
        ("status-right", String::new()),
        ("status-format[0]", statusline_cmd),
        ("prefix", "None".to_string()),
        ("escape-time", "0".to_string()),
        ("mouse", "on".to_string()),
        ("allow-passthrough", "on".to_string()),
        ("focus-events", "on".to_string()),
        ("default-terminal", "tmux-256color".to_string()),
        ("terminal-overrides", ",*:Tc,RGB,extkeys".to_string()),
    ];

    for (opt, val) in &options {
        let status = std::process::Command::new("tmux")
            .args(["set-option", "-t", session, opt, val])
            .status()
            .with_context(|| format!("tmux set-option {opt}"))?;
        if !status.success() && !matches!(*opt, "allow-passthrough" | "terminal-overrides") {
            anyhow::bail!("tmux set-option {opt} {val} failed for session {session}");
        }
    }

    Ok(())
}

pub(super) fn default_statusline_cmd(bin: &str) -> String {
    format!("#({bin} harness statusline --tmux #{{?@te_session,--session #{{q:@te_session}},}})")
}

/// Spawn a new tmux window running `slug`'s harness in `project`'s directory.
/// Returns the new pane id (e.g. "%7") or an error.
#[allow(clippy::too_many_arguments)]
pub async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    launch_args: Vec<String>,
    base_override: Option<Vec<String>>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
    ordinal: Option<u32>,
) -> Result<String> {
    if !tmux_available() {
        anyhow::bail!("tmux binary not found");
    }

    let (base_command, agent_def) = match base_override {
        Some(cmd) => (cmd, None),
        None => resolve_spawn_entry(slug)?,
    };
    let mut agent_command = apply_agent_def_args(base_command, slug, agent_def);
    if !launch_args.is_empty() {
        agent_command.extend(launch_args);
    }
    let def = find_spawn_def(slug);
    let window_name_owned: String;
    let window_name: &str = match def {
        Some(d) => d.window_name,
        None => {
            window_name_owned = format!("{}·tenex-edge", slug);
            &window_name_owned
        }
    };

    let abs_path = project_abs_path(state, project, client_cwd)?;
    open_agent_session(slug, window_name, &abs_path, &agent_command, group, ordinal).await
}

/// Resume a prior session by replaying its harness with the native resume token.
pub async fn resume_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    resume_id: &str,
) -> Result<String> {
    resume_agent_in_channel(state, slug, project, project, resume_id).await
}

/// Resume a prior session into an explicit channel while using `project` to
/// resolve the working directory.
pub async fn resume_agent_in_channel(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    group: &str,
    resume_id: &str,
) -> Result<String> {
    if !tmux_available() {
        anyhow::bail!("tmux binary not found");
    }
    if resume_id.is_empty() {
        anyhow::bail!("session has no resume token (not resumable)");
    }

    let (base, _agent_def) = resolve_spawn_entry(slug)?;
    let bin = base.first().map(String::as_str).unwrap_or("");
    let shape = resume_shape_for_bin(bin).with_context(|| {
        format!("don't know how to resume harness binary {bin:?} (agent {slug:?})")
    })?;
    let resume_command = build_resume_command(&base, shape, resume_id);

    let window_name = format!("{slug}·resume");
    let abs_path = project_abs_path(state, project, None)?;
    // ordinal=None: a resumed claude/codex session re-registers under the SAME
    // session_id, so select_session_signer recovers its ordinal from the existing
    // (pubkey,h) route — no explicit hint needed.
    open_agent_session(
        slug,
        &window_name,
        &abs_path,
        &resume_command,
        Some(group),
        None,
    )
    .await
}
