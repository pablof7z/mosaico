use super::HostDef;
use anyhow::{Context, Result};
use std::path::Path;

// ── normalized observation reporting ────────────────────────────────────────

/// Report a NORMALIZED session observation to the daemon and return the
/// canonical session id the daemon resolved for it.
///
/// Hooks no longer decide identity: they describe what they observed (harness,
/// the harness-owned external id if any, the resume token, hosted PTY session,
/// watched pid, cwd) and the daemon resolves the canonical id — minting a new one,
/// reattaching to an existing one via an alias, or superseding a stale one.
///
/// `harness_session_id` is `Some` only for harnesses that own an id of their own
/// (claude-code, codex); it is `None` for programmatic hosts (opencode) whose
/// only stable anchors are the resume token / hosted PTY session / watched pid. Each
/// present locator becomes a session alias the registry uses to reattach future
/// starts to the same canonical id.
///
/// `pty_session` is read from the hook's environment so the daemon can register
/// a local attach/injection endpoint and reattach a resumed session. Its PTY
/// metadata owns the socket path.
pub(super) async fn report_observation(
    host: &HostDef,
    agent_slug: &str,
    cwd: &Path,
    harness_session_id: Option<String>,
    resume_id: Option<String>,
    watch_pid: Option<i32>,
    provision_command: Option<Vec<String>>,
) -> Result<String> {
    let pty_session = std::env::var("TENEX_EDGE_PTY_SESSION")
        .ok()
        .filter(|s| !s.is_empty());
    let durable_reservation = std::env::var("TENEX_EDGE_DURABLE_RESERVATION")
        .ok()
        .filter(|s| !s.is_empty());
    let session_name = std::env::var("TENEX_EDGE_SESSION_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let params = serde_json::json!({
        "agent": agent_slug,
        "harness": host.name,
        "session_id": harness_session_id,
        "resume_id": resume_id,
        "cwd": cwd.to_string_lossy(),
        "watch_pid": watch_pid,
        "pty_session": pty_session,
        "durable_reservation": durable_reservation,
        "session_name": session_name,
        // Real argv of a direct `claude --agent <slug>` invocation, detected
        // when TENEX_EDGE_AGENT was absent. Only used by the daemon to seed a
        // brand-new identity's spawn command; ignored for an existing one.
        "provision_command": provision_command,
        // NIP-29 subgroup id this hosted process was spawned into, when
        // present. The daemon stores the session under this `h` instead of the
        // cwd-derived channel so its presence/chat publish into the subgroup.
        "channel": crate::cli::channel_env(),
    });
    let v = super::super::daemon_call_hook_async_with_items("session_start", params, |item| {
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

fn ps_args(pid: i32) -> Option<String> {
    std::process::Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extract the value of a `--agent <name>` / `--agent=<name>` flag from an
/// already-split argv. Does not match `--agents` (the inline-agent-defs
/// flag).
fn extract_agent_flag(argv: &[String]) -> Option<String> {
    argv.iter().enumerate().find_map(|(i, a)| {
        if a == "--agent" {
            argv.get(i + 1).cloned()
        } else {
            a.strip_prefix("--agent=").map(str::to_string)
        }
    })
}

/// Look for a live ancestor directly running `claude ... --agent <name>` —
/// i.e. NOT spawned via `tenex-edge launch`, which sets `TENEX_EDGE_AGENT`
/// instead and short-circuits this search. Returns the requested slug and
/// the ancestor's full command line (split into argv), so a brand-new
/// identity can be provisioned with that exact invocation as its spawn
/// command.
pub(super) fn find_direct_agent_invocation() -> Option<(String, Vec<String>)> {
    let mut pid = std::process::id() as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        let ppid = ps_ppid(pid)?;
        if ppid <= 1 || !seen.insert(ppid) {
            return None;
        }
        if ps_comm(ppid).to_lowercase().contains("claude") {
            if let Some(args) = ps_args(ppid) {
                let argv = shlex::split(&args).unwrap_or_else(|| vec![args.clone()]);
                if let Some(slug) = extract_agent_flag(&argv) {
                    return Some((slug, argv));
                }
            }
        }
        pid = ppid;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::extract_agent_flag;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn extract_agent_flag_finds_space_separated_form() {
        let a = argv(&["claude", "--agent", "chief-of-staff"]);
        assert_eq!(extract_agent_flag(&a).as_deref(), Some("chief-of-staff"));
    }

    #[test]
    fn extract_agent_flag_finds_equals_form() {
        let a = argv(&["claude", "--agent=chief-of-staff"]);
        assert_eq!(extract_agent_flag(&a).as_deref(), Some("chief-of-staff"));
    }

    #[test]
    fn extract_agent_flag_ignores_agents_flag() {
        let a = argv(&["claude", "--agents", r#"{"x":1}"#]);
        assert_eq!(extract_agent_flag(&a), None);
    }

    #[test]
    fn extract_agent_flag_absent_when_no_flag() {
        let a = argv(&["claude", "--dangerously-skip-permissions"]);
        assert_eq!(extract_agent_flag(&a), None);
    }

    #[test]
    fn extract_agent_flag_dangling_flag_at_end_yields_none() {
        let a = argv(&["claude", "--agent"]);
        assert_eq!(extract_agent_flag(&a), None);
    }
}
