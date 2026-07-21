use super::*;
use std::time::Instant;

mod args;
mod render;

pub(super) use args::{who, WhoArgs};

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current workspace the
/// same way the old CLI did (`all_workspaces ? None : resolve(cwd)`).
fn who_params(workspace: &Option<String>, all_workspaces: bool) -> serde_json::Value {
    crate::cli::rpc_params(serde_json::json!({
        "workspace": workspace,
        "all_workspaces": all_workspaces,
        "human_color": stdout_color_enabled(),
    }))
}

fn who_value_via_daemon(
    workspace: &Option<String>,
    all_workspaces: bool,
) -> Result<serde_json::Value> {
    crate::daemon::blocking::call("who", who_params(workspace, all_workspaces))
}

/// `who --expired`: fetch this machine's dead/old sessions from the daemon and
/// render them (public handle + channel + last_seen + resumable) for resume.
fn who_expired() -> Result<()> {
    let v = crate::daemon::blocking::call(
        "who",
        crate::cli::rpc_params(serde_json::json!({ "expired": true })),
    )?;
    let rows: Vec<crate::expired_sessions::ExpiredSessionRow> = v
        .get("expired")
        .cloned()
        .map(serde_json::from_value)
        .transpose()?
        .unwrap_or_default();
    print!("{}", render::render_expired(&rows));
    Ok(())
}

fn who_once(workspace: Option<String>, all_workspaces: bool) -> Result<()> {
    let v = who_value_via_daemon(&workspace, all_workspaces)?;
    let human = v["fabric_human"]
        .as_str()
        .context("who response missing human fabric view")?;
    print!("{human}");
    Ok(())
}

fn who_live(workspace: Option<String>, all_workspaces: bool) -> Result<()> {
    let refresh = Duration::from_millis(1000);
    let _terminal = render::LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let v = who_value_via_daemon(&workspace, all_workspaces)?;
            let human = v["fabric_human"]
                .as_str()
                .context("who response missing human fabric view")?;
            render::draw_fabric_live(human, refresh)?;
            next_draw = Instant::now() + refresh;
        }

        let wait = next_draw
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        if event::poll(wait)? && render::should_quit_live(event::read()?) {
            break;
        }
    }

    Ok(())
}

fn stdout_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

#[cfg(test)]
mod tests;
