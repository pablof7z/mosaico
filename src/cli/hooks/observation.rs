use super::HostDef;
use anyhow::{Context, Result};
use std::path::Path;

// ── normalized observation reporting ────────────────────────────────────────

/// Report a NORMALIZED session observation to the daemon and return the
/// canonical session id the daemon resolved for it.
///
/// Hooks no longer decide identity: they describe what they observed (harness,
/// the harness-owned external id if any, the resume token, tmux pane, watched
/// pid, cwd) and the daemon resolves the canonical id — minting a new one,
/// reattaching to an existing one via an alias, or superseding a stale one.
///
/// `harness_session_id` is `Some` only for harnesses that own an id of their own
/// (claude-code, codex); it is `None` for programmatic hosts (opencode) whose
/// only stable anchors are the resume token / tmux pane / watched pid. Each
/// present locator becomes a session alias the registry uses to reattach future
/// starts to the same canonical id.
///
/// `tmux_pane` / `tmux_socket` are read from the hook's environment so the daemon
/// can register a tmux endpoint and (via the pane) reattach a resumed session.
pub(super) async fn report_observation(
    host: &HostDef,
    agent_slug: &str,
    cwd: &Path,
    harness_session_id: Option<String>,
    resume_id: Option<String>,
    watch_pid: Option<i32>,
) -> Result<String> {
    let tmux_pane = std::env::var("TMUX_PANE").ok().filter(|s| !s.is_empty());
    let tmux_socket = std::env::var("TMUX").ok().filter(|s| !s.is_empty());
    let params = serde_json::json!({
        "agent": agent_slug,
        "harness": host.name,
        "harness_session_id": harness_session_id,
        "resume_id": resume_id,
        "cwd": cwd.to_string_lossy(),
        "watch_pid": watch_pid,
        "tmux_pane": tmux_pane,
        "tmux_socket": tmux_socket,
        // NIP-29 subgroup id this pane was spawned into (TENEX_EDGE_CHANNEL), when
        // present. The daemon stores the session under this `h` instead of the
        // cwd-derived project so its presence/chat publish into the subgroup.
        "channel": crate::cli::channel_env(),
    });
    let v = super::super::daemon_call_async_with_items("session_start", params, |item| {
        render_init_progress(&item);
    })
    .await?;
    let sid = v["session_id"]
        .as_str()
        .context("daemon returned no session_id")?
        .to_string();
    Ok(sid)
}

pub(super) fn render_init_progress(item: &serde_json::Value) {
    if std::env::var("TENEX_EDGE_INIT_PROGRESS").ok().as_deref() == Some("0") {
        return;
    }
    if item.get("kind").and_then(|v| v.as_str()) != Some("init_progress") {
        return;
    }
    let elapsed = item
        .get("elapsed_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or_default();
    let phase = item.get("phase").and_then(|v| v.as_str()).unwrap_or("init");
    let message = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
    eprintln!("[tenex-edge init +{elapsed}ms] {phase}: {message}");
}

// ── process-tree PID search (for harnesses like Codex that omit their PID) ───

/// Walk the process tree upward looking for an ancestor whose command name
/// contains `needle` (case-insensitive). Returns the first match.
pub(super) fn find_ancestor_pid(needle: &str) -> Option<i32> {
    let needle = needle.to_lowercase();
    let mut pid = std::process::id() as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        let ppid = ps_ppid(pid)?;
        if ppid <= 1 || !seen.insert(ppid) {
            return None;
        }
        if ps_comm(ppid).to_lowercase().contains(&needle) {
            return Some(ppid);
        }
        pid = ppid;
    }
    None
}

fn ps_ppid(pid: i32) -> Option<i32> {
    std::process::Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
}

fn ps_comm(pid: i32) -> String {
    std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}
