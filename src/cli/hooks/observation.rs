use super::HostDef;
use anyhow::{Context, Result};
use std::path::Path;

// ── normalized observation reporting ────────────────────────────────────────

/// Report a normalized session observation to the daemon and return its
/// authoritative public key.
///
/// Hooks no longer decide identity: they describe what they observed (harness,
/// the harness-owned external locator if any, the resume token, hosted PTY
/// session, watched pid, cwd) and the daemon resolves the private run internally.
///
/// `harness_session_id` is `Some` only for harnesses that own an id of their own
/// (claude-code, codex, kimi, pi); programmatic hosts use other stable anchors. Each
/// present locator becomes a session alias the registry uses to reattach future
/// starts to the same canonical id.
///
/// `pty_session` is read from the hook's environment so the daemon can register
/// a local attach/injection endpoint and reattach a resumed session. Its PTY
/// metadata owns the socket path.
#[allow(clippy::too_many_arguments)]
pub(super) async fn report_observation(
    host: &HostDef,
    observed_harness: &str,
    agent_slug: &str,
    cwd: &Path,
    harness_session_id: Option<String>,
    resume_id: Option<String>,
    watch_pid: Option<i32>,
    profile: Option<String>,
) -> Result<String> {
    let pty_session = std::env::var("MOSAICO_PTY_SESSION")
        .ok()
        .filter(|s| !s.is_empty());
    let pubkey = std::env::var("MOSAICO_PUBKEY")
        .ok()
        .filter(|s| !s.is_empty());
    let session_name = std::env::var("MOSAICO_SESSION_NAME")
        .ok()
        .filter(|s| !s.is_empty());
    let params = serde_json::json!({
        "agent": agent_slug,
        "claimed_harness": host.name,
        "observed_harness": observed_harness,
        "admitted_transport": pty_session.as_ref().map(|_| "pty"),
        "endpoint_provenance": "hook",
        "harness_session": harness_session_id,
        "resume_id": resume_id,
        "cwd": cwd.to_string_lossy(),
        "watch_pid": watch_pid,
        "pty_session": pty_session,
        "pubkey": pubkey,
        "endpoint_kind": pty_session.as_ref().map(|_| "pty"),
        "session_name": session_name,
        // A direct `claude --agent <profile>` observation seeds only the
        // agent-owned profile selection. Launch argv remains driver/preset-owned.
        "profile": profile,
    });
    let v = super::super::daemon_call_hook_async_with_items("session_start", params, |item| {
        render_init_progress(&item);
    })
    .await?;
    let pubkey = v["pubkey"]
        .as_str()
        .context("daemon returned no pubkey")?
        .to_string();
    Ok(pubkey)
}

pub(super) fn render_init_progress(item: &serde_json::Value) {
    if std::env::var("MOSAICO_INIT_PROGRESS").ok().as_deref() == Some("0") {
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
    eprintln!("[mosaico init +{elapsed}ms] {phase}: {message}");
}

/// Identify the nearest supported harness ancestor independently of the hook
/// adapter's host claim. Returning `None` is intentional: callers must diagnose
/// missing observation instead of inferring a harness from payload shape.
pub(super) fn find_ancestor_harness() -> Option<(&'static str, i32)> {
    let mut pid = std::process::id() as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        let ppid = ps_ppid(pid)?;
        if ppid <= 1 || !seen.insert(ppid) {
            return None;
        }
        if let Some(harness) = harness_for_process(ppid) {
            return Some((harness, ppid));
        }
        pid = ppid;
    }
    None
}

pub(super) fn harness_for_process(pid: i32) -> Option<&'static str> {
    let command = ps_comm(pid).to_lowercase();
    let args = ps_args(pid).unwrap_or_default().to_lowercase();
    harness_from_process(&command, &args)
}

fn harness_from_process(command: &str, args: &str) -> Option<&'static str> {
    if command.contains("claude") || args.contains("claude-agent-acp") {
        Some("claude-code")
    } else if command.contains("codex") {
        Some("codex")
    } else if command.contains("opencode") {
        Some("opencode")
    } else if command.contains("grok") {
        Some("grok")
    } else if command.contains("goose") {
        Some("goose")
    } else if command.contains("hermes") || python_script_is_hermes(args) {
        Some("hermes")
    } else if command.contains("kimi") || node_script_is_kimi(args) {
        Some("kimi")
    } else if executable_basename(command) == "pi"
        || command.contains("pi-coding-agent")
        || node_script_is_pi(args)
    {
        Some("pi")
    } else {
        None
    }
}

fn executable_basename(command: &str) -> &str {
    std::path::Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn python_script_is_hermes(args: &str) -> bool {
    fn basename(value: &str) -> &str {
        std::path::Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
    }
    let Some(argv) = shlex::split(args) else {
        return false;
    };
    argv.first()
        .is_some_and(|value| basename(value).contains("python"))
        && argv
            .get(1)
            .is_some_and(|value| matches!(basename(value), "hermes" | "hermes-acp"))
}

fn node_script_is_kimi(args: &str) -> bool {
    args.contains("/@moonshot-ai/kimi-code/")
}

fn node_script_is_pi(args: &str) -> bool {
    args.contains("/@earendil-works/pi-coding-agent/")
        || args.contains("/pi-coding-agent/dist/cli.js")
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

#[cfg(test)]
#[path = "observation/kimi_tests.rs"]
mod kimi_tests;

/// Look for a live ancestor directly running a supported native
/// `--agent <name>` selector — i.e. NOT spawned through Mosaico, which sets
/// `MOSAICO_AGENT` instead and short-circuits this search. Returns the selected
/// profile so a brand-new identity can retain it.
pub(super) fn find_direct_agent_invocation(expected_harness: &str) -> Option<String> {
    let mut pid = std::process::id() as i32;
    let mut seen = std::collections::HashSet::new();
    for _ in 0..16 {
        let ppid = ps_ppid(pid)?;
        if ppid <= 1 || !seen.insert(ppid) {
            return None;
        }
        let command = ps_comm(ppid);
        let matches_expected = (expected_harness == "claude-code"
            && command.to_lowercase().contains("claude"))
            || harness_for_process(ppid) == Some(expected_harness);
        if matches_expected {
            if let Some(args) = ps_args(ppid) {
                let argv = shlex::split(&args).unwrap_or_else(|| vec![args.clone()]);
                if let Some(slug) = extract_agent_flag(&argv) {
                    return Some(slug);
                }
            }
        }
        pid = ppid;
    }
    None
}

#[cfg(test)]
#[path = "observation/tests.rs"]
mod tests;
