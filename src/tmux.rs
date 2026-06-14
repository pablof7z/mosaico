//! TMUX control plane for tenex-edge.
//!
//! Two public surfaces:
//!
//!   • `ring_doorbells(state)` — called after every `mention_notify.notify_waiters()`.
//!     Finds sessions that have unread inbox rows + a live tmux endpoint + no armed
//!     waiter, then injects the nudge text into the pane.
//!
//!   • `spawn_agent(state, slug, project)` — spawns a new tmux window running the
//!     appropriate harness command, and registers the new pane as a "pending spawn"
//!     so that when the harness fires its `session-start` hook, the daemon can inject
//!     the actual spawn prompt (`tenex-edge inbox` by default) rather than a generic
//!     doorbell nudge.
//!
//! Fail-open everywhere: if the `tmux` binary is absent, TMUX_PANE was never set,
//! or any sub-command errors, we log to stderr (debug only) and return Ok(()).

use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Default first-prompt text injected into a freshly-spawned pane when the
/// harness reaches its input prompt.
const SPAWN_PROMPT_DEFAULT: &str = "tenex-edge inbox";

/// How long to wait after `session_start` fires before typing into the pane.
/// The hook fires early in harness startup; we need to wait until the input
/// box is actually interactive.
const SPAWN_PROMPT_DELAY_MS: u64 = 2000;

// ── constants ─────────────────────────────────────────────────────────────────

/// Don't re-inject into the same session within this window (seconds).
const DOORBELL_DEBOUNCE_SECS: u64 = 20;

/// Text injected as the doorbell nudge (without the trailing Enter).
const DOORBELL_TEXT: &str =
    "You have new tenex-edge mentions. Run `tenex-edge inbox` to read and reply.";

// ── spawn-def registry ────────────────────────────────────────────────────────
//
// Adding a new harness: add one entry to SPAWN_DEFS. No branching needed.

struct SpawnDef {
    /// Harness slug (matches agent_slug / TENEX_EDGE_AGENT).
    slug: &'static str,
    /// Window name shown in the tmux status bar.
    window_name: &'static str,
    /// Command to run (first word of the exec, plus args).
    command: &'static [&'static str],
    /// First prompt to type once the harness reaches its input box after startup.
    /// `None` means use `SPAWN_PROMPT_DEFAULT` ("tenex-edge inbox").
    /// Useful for harnesses that need a different invocation to drain their inbox.
    spawn_prompt: Option<&'static str>,
}

static SPAWN_DEFS: &[SpawnDef] = &[
    SpawnDef {
        slug: "claude",
        window_name: "claude·tenex-edge",
        command: &["claude"],
        spawn_prompt: None,
    },
    SpawnDef {
        slug: "codex",
        window_name: "codex·tenex-edge",
        command: &["codex"],
        spawn_prompt: None,
    },
    SpawnDef {
        slug: "opencode",
        window_name: "opencode·tenex-edge",
        command: &["opencode"],
        spawn_prompt: None,
    },
];

fn find_spawn_def(slug: &str) -> Option<&'static SpawnDef> {
    SPAWN_DEFS.iter().find(|d| d.slug == slug)
}

// ── spawnable-agents query ─────────────────────────────────────────────────

/// Returns `(slug, display_command)` pairs for agents tenex-edge has an
/// identity for. The harness command comes from the agent file; SPAWN_DEFS is
/// the fallback for agents that predate the `command` field. Agents with
/// neither are omitted. Returns an empty vec when tmux is absent.
pub fn spawnable_agents() -> Vec<(String, String)> {
    if !tmux_available() {
        eprintln!("[tenex-edge] spawnable_agents: tmux not available");
        return Vec::new();
    }
    let edge_home = crate::config::edge_home();
    let agents = crate::identity::list_local_agents(&edge_home);
    eprintln!(
        "[tenex-edge] spawnable_agents: {} agents in store",
        agents.len()
    );
    let result: Vec<(String, String)> = agents
        .into_iter()
        .filter_map(|(slug, file_cmd)| {
            let display = file_cmd
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| c.join(" "))
                .or_else(|| find_spawn_def(&slug).map(|d| d.command.join(" ")));
            eprintln!("[tenex-edge] spawnable_agents: slug={slug:?} display={display:?}");
            Some((slug, display?))
        })
        .collect();
    eprintln!("[tenex-edge] spawnable_agents: result={result:?}");
    result
}

// ── in-memory debounce + armed-waiter tracking ────────────────────────────────
//
// These live on `DaemonState` as type-erased `dyn Any` would be ugly; instead we
// keep them in a module-level `OnceLock<Mutex<…>>` keyed by session_id. The
// daemon is single-process, so this is fine.

use std::sync::Mutex;
use std::sync::OnceLock;

static DEBOUNCE: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
static ARMED_WAITERS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn debounce() -> &'static Mutex<HashMap<String, u64>> {
    DEBOUNCE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn armed() -> &'static Mutex<HashMap<String, usize>> {
    ARMED_WAITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── pending-spawn tracking ────────────────────────────────────────────────────
//
// When `spawn_agent` creates a new tmux window it registers the returned pane id
// here (keyed by pane_id, value is the spawn prompt text + optional mention).
// When the harness later fires its `session-start` hook and calls
// `rpc_session_start`, the server consumes the entry: writes the mention to the
// new session's inbox (if present), and injects the prompt.

/// A triggering mention that should be pre-loaded into the spawned session's
/// inbox before the harness receives its first prompt.
pub struct PendingMention {
    pub event_id: String,
    pub from_pubkey: String,
    pub from_slug: String,
    pub from_session: String,
    pub project: String,
    pub body: String,
    pub created_at: u64,
}

/// State registered for a pane created via `spawn_agent`.
pub struct PendingSpawn {
    /// First prompt to inject once the harness reaches its input box.
    pub prompt: String,
    /// If `Some`, write this inbox row before injecting the prompt so the
    /// agent finds the triggering message when it runs `tenex-edge inbox`.
    pub mention: Option<PendingMention>,
}

/// Map from pane_id → `PendingSpawn` for panes created via `spawn_agent`
/// whose harness has not yet called `session_start`.
static PENDING_SPAWNS: OnceLock<Mutex<HashMap<String, PendingSpawn>>> = OnceLock::new();

fn pending_spawns() -> &'static Mutex<HashMap<String, PendingSpawn>> {
    PENDING_SPAWNS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register `pane_id` as a pending-spawn with the given prompt text (no
/// triggering mention).  Called by `spawn_agent` immediately after the window
/// is created.
pub fn register_pending_spawn(pane_id: String, prompt: String) {
    pending_spawns().lock().unwrap().insert(
        pane_id,
        PendingSpawn {
            prompt,
            mention: None,
        },
    );
}

/// Attach a triggering mention to a pane that was already registered via
/// `register_pending_spawn`.  If no entry exists yet (race), creates one with
/// the default prompt so the mention is not lost.
/// Called from `rpc_send_message` after `spawn_agent` returns the pane id.
pub fn register_pending_spawn_with_mention(pane_id: &str, mention: PendingMention) {
    let mut m = pending_spawns().lock().unwrap();
    let entry = m
        .entry(pane_id.to_string())
        .or_insert_with(|| PendingSpawn {
            prompt: SPAWN_PROMPT_DEFAULT.to_string(),
            mention: None,
        });
    entry.mention = Some(mention);
}

/// Remove and return the `PendingSpawn` for `pane_id`, or `None` if this pane
/// was not created by `spawn_agent` (i.e. it is a normal harness start).
/// Called by `rpc_session_start` when a pane registers its tmux endpoint.
pub fn consume_pending_spawn(pane_id: &str) -> Option<PendingSpawn> {
    pending_spawns().lock().unwrap().remove(pane_id)
}

/// Called by `handle_wait_for_mention` when it parks on `mention_notify`.
/// Prevents the doorbell from firing while a waiter is parked.
pub fn arm_waiter(session_id: &str) {
    *armed()
        .lock()
        .unwrap()
        .entry(session_id.to_string())
        .or_insert(0) += 1;
}

/// Called by `handle_wait_for_mention` when it returns (mention delivered or
/// timed out).
pub fn disarm_waiter(session_id: &str) {
    let mut m = armed().lock().unwrap();
    if let Some(n) = m.get_mut(session_id) {
        *n = n.saturating_sub(1);
        if *n == 0 {
            m.remove(session_id);
        }
    }
}

fn is_armed(session_id: &str) -> bool {
    *armed().lock().unwrap().get(session_id).unwrap_or(&0) > 0
}

fn is_debounced(session_id: &str) -> bool {
    let m = debounce().lock().unwrap();
    m.get(session_id)
        .map(|&t| now_secs().saturating_sub(t) < DOORBELL_DEBOUNCE_SECS)
        .unwrap_or(false)
}

fn record_doorbell(session_id: &str) {
    debounce()
        .lock()
        .unwrap()
        .insert(session_id.to_string(), now_secs());
}

// ── tmux binary check ─────────────────────────────────────────────────────────

fn tmux_available() -> bool {
    std::process::Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── pane liveness ─────────────────────────────────────────────────────────────

/// Verify that `pane_id` (e.g. "%5") is still alive.
/// Returns the current command running in the pane on success (e.g. "claude").
pub fn pane_alive_pub(pane_id: &str) -> Option<String> {
    pane_alive(pane_id)
}

fn pane_alive(pane_id: &str) -> Option<String> {
    let out = std::process::Command::new("tmux")
        .args([
            "display",
            "-p",
            "-t",
            pane_id,
            "#{pane_id} #{pane_current_command}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let s = s.trim();
    // Output is "<pane_id> <command>". A gone pane produces no output.
    if s.is_empty() {
        return None;
    }
    // Return just the command part.
    let cmd = s
        .split_once(' ')
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .to_string();
    Some(cmd)
}

// ── low-level tmux input helpers ──────────────────────────────────────────────

/// Send literal `text` to `pane_id` without submitting (no Enter).
/// Uses `-l` (literal paste) so special characters are not interpreted by the
/// shell or TUI.
async fn inject_text(pane_id: &str, text: &str) -> Result<()> {
    let status = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", pane_id, "-l", "--", text])
        .status()
        .await
        .context("tmux send-keys text")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys text failed for pane {pane_id}");
    }
    Ok(())
}

/// Send a bare Enter keystroke to `pane_id` to submit whatever is on the
/// input line.
async fn send_enter(pane_id: &str) -> Result<()> {
    let status = tokio::process::Command::new("tmux")
        .args(["send-keys", "-t", pane_id, "Enter"])
        .status()
        .await
        .context("tmux send-keys Enter")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys Enter failed for pane {pane_id}");
    }
    Ok(())
}

// ── doorbell injection ────────────────────────────────────────────────────────

/// Public wrapper for manual CLI invocation (tmux_rpc).
pub async fn inject_doorbell_pub(pane_id: &str) -> Result<()> {
    inject_doorbell(pane_id).await
}

/// Send the doorbell nudge to `pane_id`.
async fn inject_doorbell(pane_id: &str) -> Result<()> {
    inject_text(pane_id, DOORBELL_TEXT).await?;
    // Short pause so the TUI has time to absorb the paste.
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(())
}

// ── spawn-prompt injection ────────────────────────────────────────────────────

/// Inject `text` + Enter into `pane_id` after a startup grace delay, so that
/// a freshly-spawned harness receives its first prompt once its input box is
/// interactive.
///
/// Called from `rpc_session_start` (in a background task) when it detects that
/// the registering pane was created via `spawn_agent`.
pub async fn inject_spawn_prompt(pane_id: &str, text: &str) -> Result<()> {
    // Wait for the harness to reach its interactive input prompt.  The
    // `session-start` hook fires early in startup (before the TUI is ready to
    // accept keystrokes), so we must wait a moment before sending.
    tokio::time::sleep(Duration::from_millis(SPAWN_PROMPT_DELAY_MS)).await;

    // Abort if the pane has already died (harness crashed during startup).
    if pane_alive(pane_id).is_none() {
        anyhow::bail!("pane {pane_id} died before spawn prompt could be injected");
    }

    inject_text(pane_id, text).await?;
    // Short pause so the TUI has time to absorb the paste before Enter lands.
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(())
}

// ── public API ────────────────────────────────────────────────────────────────

/// Called after every `mention_notify.notify_waiters()`.
/// Scans for sessions with unread inbox rows that have a live tmux endpoint,
/// no armed waiter, and haven't been doorbelled recently.
/// Spawns a background task so the dispatcher never blocks the RPC path.
pub fn ring_doorbells(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        if let Err(e) = ring_doorbells_inner(&state).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] doorbell error: {e:#}");
            }
        }
    });
}

async fn ring_doorbells_inner(state: &Arc<DaemonState>) -> Result<()> {
    if !tmux_available() {
        return Ok(());
    }

    // Collect sessions that have unread inbox rows AND are currently idle.
    // Skip any session where working=1 to avoid injecting a doorbell mid-turn.
    let sessions_with_inbox: Vec<String> = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| {
                s.count_unread_inbox(&rec.session_id).unwrap_or(0) > 0
                    && !s.is_session_working(&rec.session_id)
            })
            .map(|rec| rec.session_id)
            .collect()
    });

    for sid in sessions_with_inbox {
        if is_armed(&sid) || is_debounced(&sid) {
            continue;
        }

        let endpoint = state.with_store(|s| s.get_session_endpoint(&sid, "tmux"));
        let ep = match endpoint {
            Ok(Some(ep)) => ep,
            _ => continue,
        };

        let pane_id = ep.target.clone();

        // Verify pane is alive; clean up stale endpoint if not.
        if pane_alive(&pane_id).is_none() {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] pane {pane_id} gone; removing endpoint for {sid}");
            }
            state.with_store(|s| s.delete_session_endpoint(&sid, "tmux").ok());
            continue;
        }

        record_doorbell(&sid);
        let ts = now_secs();
        state.with_store(|s| s.touch_session_endpoint_verified(&sid, "tmux", ts).ok());

        if let Err(e) = inject_doorbell(&pane_id).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] doorbell inject failed for {sid} pane {pane_id}: {e:#}");
            }
        } else if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[tmux] doorbell injected into pane {pane_id} for session {sid}");
        }
    }
    Ok(())
}

// ── spawn ─────────────────────────────────────────────────────────────────────

/// Spawn a new tmux window running `slug`'s harness in `project`'s directory.
/// Returns the new pane id (e.g. "%7") or an error.
pub async fn spawn_agent(state: &Arc<DaemonState>, slug: &str, project: &str) -> Result<String> {
    if !tmux_available() {
        anyhow::bail!("tmux binary not found");
    }

    // Resolve the harness command: agent file takes priority, SPAWN_DEFS is the fallback.
    let edge_home = crate::config::edge_home();
    let file_cmd = crate::identity::list_local_agents(&edge_home)
        .into_iter()
        .find(|(s, _)| s == slug)
        .and_then(|(_, cmd)| cmd.filter(|c| !c.is_empty()));
    let agent_command: Vec<String> = file_cmd
        .or_else(|| {
            find_spawn_def(slug).map(|d| d.command.iter().map(|s| s.to_string()).collect())
        })
        .with_context(|| format!("no harness command for agent {slug:?}: add a \"command\" field to ~/.tenex/edge/agents/{slug}.json"))?;

    let def = find_spawn_def(slug); // optional, for window_name / spawn_prompt defaults
    let window_name_owned: String;
    let window_name: &str = match def {
        Some(d) => d.window_name,
        None => {
            window_name_owned = format!("{}·tenex-edge", slug);
            &window_name_owned
        }
    };

    let abs_path = state
        .with_store(|s| s.get_project_path(project))
        .unwrap_or(None)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });

    // Ensure a "tenex" session exists (detached); create if absent.
    // Use list-sessions + exact string match to avoid tmux's prefix-matching
    // semantics for `-t`, which would treat "tenex-test" as a match for "tenex".
    let session_exists = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|l| l == "tenex")
        })
        .unwrap_or(false);

    if !session_exists {
        let _ = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", "tenex", "-c", &abs_path])
            .status();
    }

    // Build the new-window command.
    //
    // The spawned agent's slug MUST travel into the pane's environment via tmux's
    // `-e` flag, NOT via `.env()` on the tmux client below: tmux builds a new
    // pane's environment from the server's environment plus `-e` overrides, so a
    // var set only on the client process that issues `new-window` is dropped and
    // never reaches the pane. The session-start hook prefers `TENEX_EDGE_AGENT`
    // over the harness's own default slug (see cli/hooks.rs), so without this the
    // spawn's known identity is lost and a `codex` (or any custom) agent registers
    // under the harness default (e.g. `claude`) — the wrong name in `who`/`tmux`.
    let agent_env = format!("TENEX_EDGE_AGENT={slug}");
    let mut cmd_args: Vec<&str> = vec![
        "new-window",
        "-d",
        "-t",
        "tenex",
        "-n",
        window_name,
        "-c",
        &abs_path,
        "-e",
        "TENEX_EDGE_SPAWNED=1",
        "-e",
        &agent_env,
        "-PF",
        "#{pane_id}",
        "--",
    ];
    let cmd_strs: Vec<&str> = agent_command.iter().map(|s| s.as_str()).collect();
    cmd_args.extend_from_slice(&cmd_strs);

    let out = tokio::process::Command::new("tmux")
        .args(&cmd_args)
        .output()
        .await
        .context("tmux new-window")?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!("tmux new-window failed: {stderr}");
    }

    let pane_id = String::from_utf8(out.stdout)
        .context("tmux new-window output")?
        .trim()
        .to_string();

    // Register as a pending spawn so that when the harness fires its
    // `session-start` hook, `rpc_session_start` injects the actual prompt
    // instead of waiting for a generic `ring_doorbells` nudge.
    let prompt = def
        .and_then(|d| d.spawn_prompt)
        .unwrap_or(SPAWN_PROMPT_DEFAULT)
        .to_string();
    register_pending_spawn(pane_id.clone(), prompt);

    Ok(pane_id)
}

// ── status query ──────────────────────────────────────────────────────────────

pub struct EndpointStatus {
    pub session_id: String,
    pub pane_id: String,
    pub pane_command: String,
    pub alive: bool,
    pub registered_at: u64,
    pub last_verified: u64,
}

/// List all registered tmux endpoints with liveness.
pub fn list_endpoint_statuses(state: &Arc<DaemonState>) -> Vec<EndpointStatus> {
    let endpoints =
        state.with_store(|s| s.list_session_endpoints_of_kind("tmux").unwrap_or_default());

    endpoints
        .into_iter()
        .map(|ep| {
            let cmd_opt = pane_alive(&ep.target);
            EndpointStatus {
                session_id: ep.session_id,
                pane_id: ep.target,
                pane_command: cmd_opt.clone().unwrap_or_default(),
                alive: cmd_opt.is_some(),
                registered_at: ep.registered_at,
                last_verified: ep.last_verified,
            }
        })
        .collect()
}
