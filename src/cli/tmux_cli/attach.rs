/// Attach helpers for tmux pane/session management
use anyhow::Context as _;

// ── shared attach logic ───────────────────────────────────────────────────────

/// Resolve a session id to its live tmux pane id via the daemon, or `None`.
pub(super) fn pane_for_session(session_id: &str) -> Option<String> {
    let v =
        crate::daemon::blocking::call("tmux_attach", serde_json::json!({ "session": session_id }))
            .ok()?;
    v["pane_id"].as_str().map(str::to_string)
}

pub(super) fn attach_session(session_id: &str) -> anyhow::Result<()> {
    let v =
        crate::daemon::blocking::call("tmux_attach", serde_json::json!({ "session": session_id }))
            .context("tmux_attach RPC")?;

    let pane_id = match v["pane_id"].as_str() {
        Some(p) => p.to_string(),
        None => {
            let err = v["error"].as_str().unwrap_or("unknown error");
            eprintln!("Cannot attach: {err}");
            return Ok(());
        }
    };

    attach_pane(&pane_id)
}

/// Resolve a pane id (e.g. "%7") to `(session_name, window_index)` by scanning
/// every pane in every session. Returns `None` if the pane is gone.
pub(super) fn resolve_pane_location(pane_id: &str) -> Option<(String, String)> {
    let out = std::process::Command::new("tmux")
        .args([
            "list-panes",
            "-a",
            "-F",
            "#{pane_id} #{session_name} #{window_index}",
        ])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    stdout.lines().find_map(|line| {
        let mut parts = line.splitn(3, ' ');
        let pid = parts.next()?;
        let session = parts.next()?;
        let window = parts.next()?;
        if pid == pane_id {
            Some((session.to_string(), window.to_string()))
        } else {
            None
        }
    })
}

/// The tmux session that owns `pane_id` (one session per agent now), or `None`
/// if the pane is gone.
pub(super) fn session_of_pane(pane_id: &str) -> Option<String> {
    resolve_pane_location(pane_id).map(|(session, _window)| session)
}

/// Attach to the session owning `pane_id` as a BLOCKING child, returning when the
/// user detaches (Ctrl-b d) or the session ends. `$TMUX` is stripped from the
/// child so it works even when the caller is itself inside tmux (nested attach) —
/// this is what lets the `tenex-edge tmux` TUI stay running underneath and be
/// returned to afterward. No grouped "view" session is needed: each agent is its
/// own single-window session, so there is no current-window pointer to mirror.
pub(super) fn attach_pane_blocking(pane_id: &str) -> anyhow::Result<()> {
    let Some(session) = session_of_pane(pane_id) else {
        anyhow::bail!("pane {pane_id} not found in any tmux session");
    };
    std::process::Command::new("tmux")
        .args(["attach-session", "-t", &session])
        .env_remove("TMUX")
        .status()
        .context("tmux attach-session")?;
    Ok(())
}

/// Attach by replacing this process (for the one-shot CLI verbs, where returning
/// to a shell on detach is the right behavior). Inside tmux it switches the
/// current client; outside it execs `attach-session`.
pub(super) fn attach_pane(pane_id: &str) -> anyhow::Result<()> {
    let Some(session) = session_of_pane(pane_id) else {
        eprintln!("Pane {pane_id} not found in any tmux session.");
        return Ok(());
    };

    let in_tmux = std::env::var("TMUX")
        .map(|v| !v.is_empty())
        .unwrap_or(false);
    if in_tmux {
        let status = std::process::Command::new("tmux")
            .args(["switch-client", "-t", &session])
            .status()
            .context("tmux switch-client")?;
        if !status.success() {
            eprintln!("tmux switch-client failed for session {session}");
        }
        return Ok(());
    }

    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("tmux")
        .args(["attach-session", "-t", &session])
        .exec(); // replaces this process; only returns on error
    anyhow::bail!("exec tmux attach-session: {err}");
}

/// Ask the daemon to resume `session`, returning the new pane id (or `None`,
/// after printing the error). Shared by the CLI verb and the TUI.
pub(super) fn resume_to_pane(session: &str) -> anyhow::Result<Option<String>> {
    let v = crate::daemon::blocking::call("tmux_resume", serde_json::json!({ "session": session }))
        .context("tmux_resume RPC")?;
    match v["pane_id"].as_str() {
        Some(p) => Ok(Some(p.to_string())),
        None => {
            let err = v["error"].as_str().unwrap_or("unknown error");
            eprintln!("Cannot resume: {err}");
            Ok(None)
        }
    }
}
