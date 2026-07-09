use super::*;

mod args;
mod render;

pub(super) use args::{who, WhoArgs};

// Public re-exports for the crate and cli module
use crate::who_snapshot::WhoSnapshot;

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current root channel the
/// same way the old CLI did (`all_roots ? None : resolve(cwd)`).
fn who_params(root: &Option<String>, all_roots: bool) -> serde_json::Value {
    crate::cli::rpc_params(serde_json::json!({
        "root": root,
        "all_roots": all_roots,
        "human_color": stdout_color_enabled(),
    }))
}

fn who_value_via_daemon(root: &Option<String>, all_roots: bool) -> Result<serde_json::Value> {
    crate::daemon::blocking::call("who", who_params(root, all_roots))
}

/// `who --expired`: fetch this machine's dead/old sessions from the daemon and
/// render them (codename + channel + last_seen + resumable) for resume.
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

fn who_once(root: Option<String>, all_roots: bool) -> Result<()> {
    let v = who_value_via_daemon(&root, all_roots)?;
    if let Some(human) = v.get("fabric_human").and_then(|x| x.as_str()) {
        print!("{human}");
        return Ok(());
    }
    // Prefer the unified fabric view (same format as the hook injection and as
    // single-root `who`). The daemon sets this for both a resolved current
    // channel and `--all-roots` (one root-channel block per root channel).
    if let Some(fabric) = v.get("fabric").and_then(|x| x.as_str()) {
        println!("{fabric}");
        return Ok(());
    }
    let snapshot: WhoSnapshot = serde_json::from_value(v)?;
    print!("{}", render::render_who_for_stdout(&snapshot));
    Ok(())
}

fn who_live(root: Option<String>, all_roots: bool) -> Result<()> {
    let refresh = Duration::from_millis(1000);
    let _terminal = render::LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let v = who_value_via_daemon(&root, all_roots)?;
            if let Some(human) = v.get("fabric_human").and_then(|x| x.as_str()) {
                render::draw_fabric_live(human, refresh)?;
            } else if let Some(fabric) = v.get("fabric").and_then(|x| x.as_str()) {
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

fn stdout_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && io::stdout().is_terminal()
}

#[cfg(test)]
mod tests;
