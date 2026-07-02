use super::*;

mod fabric_context;
mod render;
mod snapshot;

// Public re-exports for the crate and cli module
pub(crate) use fabric_context::{inbox_seed, render_fabric_context, FabricContextInput};
pub use snapshot::{load_who_snapshot, WhoSnapshot};

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current project the
/// same way the old CLI did (`all_projects ? None : resolve(cwd)`).
fn who_params(project: &Option<String>, all_projects: bool) -> serde_json::Value {
    crate::cli::rpc_params(serde_json::json!({
        "project": project,
        "all_projects": all_projects,
    }))
}

fn who_value_via_daemon(project: &Option<String>, all_projects: bool) -> Result<serde_json::Value> {
    crate::daemon::blocking::call("who", who_params(project, all_projects))
}

pub(super) fn who(project: Option<String>, all_projects: bool) -> Result<()> {
    let v = who_value_via_daemon(&project, all_projects)?;
    // Prefer the unified fabric view (same format as the hook injection). The
    // daemon includes it whenever a current channel resolves; `--all-projects`
    // (no single scope) falls back to the cross-project snapshot table.
    if let Some(fabric) = v.get("fabric").and_then(|x| x.as_str()) {
        println!("{fabric}");
        return Ok(());
    }
    let snapshot: WhoSnapshot = serde_json::from_value(v)?;
    print!("{}", render::render_who_for_stdout(&snapshot));
    Ok(())
}

pub(super) fn who_live(project: Option<String>, all_projects: bool) -> Result<()> {
    let refresh = Duration::from_millis(1000);
    let _terminal = render::LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let v = who_value_via_daemon(&project, all_projects)?;
            if let Some(fabric) = v.get("fabric").and_then(|x| x.as_str()) {
                render::draw_fabric_live(fabric, refresh)?;
            } else {
                let snapshot: WhoSnapshot = serde_json::from_value(v)?;
                render::draw_who_live(&snapshot, refresh)?;
            }
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

#[cfg(test)]
mod tests;
