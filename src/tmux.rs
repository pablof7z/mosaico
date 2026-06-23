//! TMUX control plane for tenex-edge.
//!
//! Two public surfaces:
//!
//!   • `ring_doorbells(state)` — called after chat delivery events.
//!     Finds sessions that have unread chat mentions + a live tmux endpoint, then
//!     injects the rendered pending messages into the pane.
//!
//!   • `spawn_agent(state, slug, project, launch_args)` — spawns a new tmux window
//!     running the appropriate harness command. Manual spawns start clean — no
//!     prompt is injected.
//!
//! Fail-open everywhere: if the `tmux` binary is absent, TMUX_PANE was never set,
//! or any sub-command errors, we log to stderr (debug only) and return Ok(()).

use crate::daemon::server::DaemonState;
use crate::util::now_secs;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// How long to wait after `session_start` fires before typing into the pane.
/// The hook fires early in harness startup; we need to wait until the input
/// box is actually interactive.
const SPAWN_PROMPT_DELAY_MS: u64 = 2000;

// ── constants ─────────────────────────────────────────────────────────────────

/// Don't re-inject into the same session within this window (seconds).
const MESSAGE_INJECT_DEBOUNCE_SECS: u64 = 20;

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
}

/// How a harness's launch command is transformed into a *resume* invocation.
/// The base command is the agent's configured launch command (e.g. `["claude",
/// "--dangerously-skip-permissions"]`), so the user's own flags are preserved.
#[derive(Clone, Copy)]
enum ResumeShape {
    /// Resume is a flag that composes with the launch flags: append `<flag> <id>`
    /// to the base command. claude: `--resume`, opencode: `--session`.
    AppendFlag(&'static str),
    /// Resume is a subcommand that must follow the binary: insert `<sub> <id>`
    /// right after argv[0], keeping the remaining launch flags after it. The
    /// flags ride on the subcommand's own parser. codex: `resume`.
    Subcommand(&'static str),
}

static SPAWN_DEFS: &[SpawnDef] = &[
    SpawnDef {
        slug: "claude",
        window_name: "claude·tenex-edge",
        command: &["claude"],
    },
    SpawnDef {
        slug: "codex",
        window_name: "codex·tenex-edge",
        command: &["codex"],
    },
    SpawnDef {
        slug: "opencode",
        window_name: "opencode·tenex-edge",
        command: &["opencode"],
    },
    SpawnDef {
        slug: "grok",
        window_name: "grok·tenex-edge",
        command: &["grok"],
    },
];

fn find_spawn_def(slug: &str) -> Option<&'static SpawnDef> {
    SPAWN_DEFS.iter().find(|d| d.slug == slug)
}

/// The resume shape for a harness, keyed by the launch command's *binary* (not
/// the agent slug): resume syntax is a property of the harness, and custom
/// agents (e.g. `developer` → `claude --dangerously-skip-permissions`) share the
/// underlying binary's resume convention. `bin` may be a path; the basename is
/// matched. Returns `None` for binaries we don't know how to resume.
fn resume_shape_for_bin(bin: &str) -> Option<ResumeShape> {
    let name = std::path::Path::new(bin)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(bin);
    match name {
        "claude" => Some(ResumeShape::AppendFlag("--resume")),
        "codex" => Some(ResumeShape::Subcommand("resume")),
        "opencode" => Some(ResumeShape::AppendFlag("--session")),
        "grok" => Some(ResumeShape::AppendFlag("--resume")),
        _ => None,
    }
}

/// Transform a base launch command into a resume invocation for `shape`.
/// Pure (no I/O) so it is unit-tested directly.
///
///   AppendFlag("--resume"):  [claude, --flag]        → [claude, --flag, --resume, <id>]
///   Subcommand("resume"):    [codex,  --flag]         → [codex, resume, <id>, --flag]
fn build_resume_command(base: &[String], shape: ResumeShape, resume_id: &str) -> Vec<String> {
    match shape {
        ResumeShape::AppendFlag(flag) => {
            let mut out = base.to_vec();
            out.push(flag.to_string());
            out.push(resume_id.to_string());
            out
        }
        ResumeShape::Subcommand(sub) => {
            let mut out = Vec::with_capacity(base.len() + 2);
            let mut it = base.iter();
            if let Some(bin) = it.next() {
                out.push(bin.clone());
            }
            out.push(sub.to_string());
            out.push(resume_id.to_string());
            out.extend(it.cloned());
            out
        }
    }
}

// ── spawnable-agents query ─────────────────────────────────────────────────

/// Returns `(slug, display_command, byline)` tuples for agents tenex-edge has
/// an identity for. The harness command comes from the agent file; SPAWN_DEFS
/// is the fallback for agents that predate the `command` field. Agents with
/// neither are omitted. `byline` is the agent's optional "when to use" note.
/// Returns an empty vec when tmux is absent.
pub fn spawnable_agents() -> Vec<(String, String, Option<String>)> {
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
    let result: Vec<(String, String, Option<String>)> = agents
        .into_iter()
        .filter_map(|(slug, file_cmd, _agent_def, byline)| {
            let display = file_cmd
                .as_ref()
                .filter(|c| !c.is_empty())
                .map(|c| c.join(" "))
                .or_else(|| find_spawn_def(&slug).map(|d| d.command.join(" ")));
            eprintln!("[tenex-edge] spawnable_agents: slug={slug:?} display={display:?}");
            Some((slug, display?, byline))
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
fn debounce() -> &'static Mutex<HashMap<String, u64>> {
    DEBOUNCE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_debounced(session_id: &str) -> bool {
    let m = debounce().lock().unwrap();
    m.get(session_id)
        .map(|&t| now_secs().saturating_sub(t) < MESSAGE_INJECT_DEBOUNCE_SECS)
        .unwrap_or(false)
}

fn record_message_injection(session_id: &str) {
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

// ── spawn-message injection ───────────────────────────────────────────────────

/// Paste `text` into `pane_id` via a tmux paste buffer in bracketed-paste mode
/// (`-p`). Unlike `send-keys -l`, this delivers embedded newlines as input
/// rather than submitting at each line break — required because the spawn
/// message is a multi-line envelope. The buffer is loaded over stdin (no
/// arg-escaping pitfalls) and deleted after the paste (`-d`).
async fn paste_text(pane_id: &str, text: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    const BUF: &str = "te-spawn-msg";

    let mut child = tokio::process::Command::new("tmux")
        .args(["load-buffer", "-b", BUF, "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .context("tmux load-buffer spawn")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .await
            .context("writing tmux paste buffer")?;
    }
    let status = child.wait().await.context("tmux load-buffer wait")?;
    if !status.success() {
        anyhow::bail!("tmux load-buffer failed for pane {pane_id}");
    }

    let status = tokio::process::Command::new("tmux")
        .args(["paste-buffer", "-p", "-d", "-b", BUF, "-t", pane_id])
        .status()
        .await
        .context("tmux paste-buffer")?;
    if !status.success() {
        anyhow::bail!("tmux paste-buffer failed for pane {pane_id}");
    }
    Ok(())
}

/// Type the received message into `pane_id` and submit it, so a freshly-spawned
/// harness opens on the message that triggered its spawn. Waits a startup grace
/// delay for the input box to become interactive, pastes the (multi-line)
/// message with bracketed paste, then sends Enter.
///
/// Called from `rpc_session_start` (in a background task) when the registering
/// pane was created by spawn-on-send.
pub async fn inject_spawn_message(pane_id: &str, text: &str) -> Result<()> {
    // The `session-start` hook fires early in startup (before the TUI is ready
    // to accept input), so wait until the input box is interactive.
    tokio::time::sleep(Duration::from_millis(SPAWN_PROMPT_DELAY_MS)).await;

    // Abort if the pane has already died (harness crashed during startup).
    if pane_alive(pane_id).is_none() {
        anyhow::bail!("pane {pane_id} died before spawn message could be injected");
    }

    paste_text(pane_id, text).await?;
    // Short pause so the TUI has time to absorb the paste before Enter lands.
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(())
}

// ── pending-message injection ─────────────────────────────────────────────────

struct PendingTmuxPrompt {
    text: String,
    chat_ids: Vec<String>,
}

fn render_pending_message_prompt(
    chat_rows: &[crate::state::ChatInboxRow],
    rec: &crate::state::SessionRecord,
    now: u64,
) -> Option<String> {
    if chat_rows.is_empty() {
        return None;
    }
    Some(crate::cli::render_chat_block(
        "tenex-edge project chat - write with `tenex-edge chat write < message.txt`; mention a session with `tenex-edge chat write --mention <session-id>`:",
        chat_rows,
        &rec.session_id,
        now,
    ))
}

fn collect_pending_prompt(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
) -> Result<Option<PendingTmuxPrompt>> {
    let now = now_secs();
    state.with_store(|s| -> Result<Option<PendingTmuxPrompt>> {
        let chat_rows = s.peek_chat_mentions(&rec.session_id)?;

        let Some(text) = render_pending_message_prompt(&chat_rows, rec, now) else {
            return Ok(None);
        };

        Ok(Some(PendingTmuxPrompt {
            text,
            chat_ids: chat_rows
                .iter()
                .map(|row| row.chat_event_id.clone())
                .collect(),
        }))
    })
}

fn mark_prompt_delivered(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
    prompt: &PendingTmuxPrompt,
) -> Result<()> {
    let delivered_at = now_secs();
    state.with_store(|s| -> Result<()> {
        s.mark_chat_rows_delivered(&rec.session_id, &prompt.chat_ids, delivered_at)?;
        Ok(())
    })
}

/// Paste pending inbox/chat content into a live pane and submit it as the next
/// prompt. Returns false if another path consumed the rows before we injected.
pub async fn inject_pending_messages_pub(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
    pane_id: &str,
) -> Result<bool> {
    let Some(prompt) = collect_pending_prompt(state, rec)? else {
        return Ok(false);
    };

    paste_text(pane_id, &prompt.text).await?;
    // Mark the exact rendered rows delivered before Enter lands, so the harness
    // turn-start hook does not drain and inject the same content a second time.
    mark_prompt_delivered(state, rec, &prompt)?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    send_enter(pane_id).await?;
    Ok(true)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Scans for sessions with unread inbox rows that have a live tmux endpoint,
/// and have not been injected recently.
/// Spawns a background task so the dispatcher never blocks the RPC path.
pub fn ring_doorbells(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        if let Err(e) = ring_doorbells_inner(&state).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] pending message injection error: {e:#}");
            }
        }
    });
}

async fn ring_doorbells_inner(state: &Arc<DaemonState>) -> Result<()> {
    if !tmux_available() {
        return Ok(());
    }

    // Collect sessions that have unread explicit chat mentions AND are currently
    // idle. Skip any session where working=1 to avoid typing a prompt mid-turn.
    let sessions_with_chat: Vec<crate::state::SessionRecord> = state.with_store(|s| {
        s.list_alive_sessions()
            .unwrap_or_default()
            .into_iter()
            .filter(|rec| {
                s.count_unread_chat_mentions(&rec.session_id).unwrap_or(0) > 0
                    && !s.is_session_working(&rec.session_id)
            })
            .collect()
    });

    for rec in sessions_with_chat {
        let sid = rec.session_id.clone();
        if is_debounced(&sid) {
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

        record_message_injection(&sid);
        let ts = now_secs();
        state.with_store(|s| s.touch_session_endpoint_verified(&sid, "tmux", ts).ok());

        match inject_pending_messages_pub(state, &rec, &pane_id).await {
            Ok(true) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[tmux] pending messages injected into pane {pane_id} for session {sid}"
                    );
                }
            }
            Ok(false) => {}
            Err(e) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[tmux] pending message inject failed for {sid} pane {pane_id}: {e:#}"
                    );
                }
            }
        }
    }
    Ok(())
}

// ── spawn ─────────────────────────────────────────────────────────────────────

/// Resolve the base harness command and inline agent definition for `slug`.
/// The agent file's `command` field takes priority, with SPAWN_DEFS as fallback.
fn resolve_spawn_entry(slug: &str) -> Result<(Vec<String>, Option<serde_json::Value>)> {
    let edge_home = crate::config::edge_home();
    let entry = crate::identity::list_local_agents(&edge_home)
        .into_iter()
        .find(|(s, _, _, _)| s == slug);
    let (file_cmd, agent_def) = entry
        .map(|(_, cmd, def, _)| (cmd.filter(|c| !c.is_empty()), def))
        .unwrap_or((None, None));
    let base = file_cmd
        .or_else(|| find_spawn_def(slug).map(|d| d.command.iter().map(|s| s.to_string()).collect()))
        .with_context(|| format!("no harness command for agent {slug:?}: add a \"command\" field to ~/.tenex-edge/agents/{slug}.json"))?;
    Ok((base, agent_def))
}

/// Append harness-specific args for the inline agent definition.
/// For `claude`: wraps the def as `{"<slug>": <def>}` and appends
/// `--agents '<json>' --agent <slug>`.
fn apply_agent_def_args(
    mut cmd: Vec<String>,
    slug: &str,
    agent_def: Option<serde_json::Value>,
) -> Vec<String> {
    let Some(def) = agent_def else { return cmd };
    let bin = cmd.first().map(String::as_str).unwrap_or("");
    match bin {
        "claude" => {
            let mut wrapper = serde_json::Map::new();
            wrapper.insert(slug.to_string(), def);
            if let Ok(json) = serde_json::to_string(&serde_json::Value::Object(wrapper)) {
                cmd.push("--agents".to_string());
                cmd.push(json);
                cmd.push("--agent".to_string());
                cmd.push(slug.to_string());
            }
        }
        _ => {}
    }
    cmd
}

/// The absolute working directory for `project`. When `client_cwd` is supplied
/// (forwarded from the `tenex-edge launch`/`tmux spawn` client), use it
/// directly and refresh the `project_paths` row so subsequent spawns without a
/// cwd still find it. Otherwise look up the project in `project_paths`, falling
/// back to the daemon's cwd.
fn project_abs_path(
    state: &Arc<DaemonState>,
    project: &str,
    client_cwd: Option<&std::path::Path>,
) -> String {
    if let Some(cwd) = client_cwd {
        let abs = cwd.to_string_lossy().to_string();
        // Refresh the project_paths row so subsequent spawns without a cwd find
        // it. Best-effort; ignore store errors.
        let now = crate::util::now_secs();
        let _ = state.with_store(|s| s.upsert_project_path(project, &abs, now));
        return abs;
    }
    state
        .with_store(|s| s.get_project_path(project))
        .unwrap_or(None)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
}

/// Pick a tmux session name `te-<slug>[-N]` that is not currently in use.
/// Uses an exact-match scan of `list-sessions` (not `has-session`, whose `-t`
/// does prefix matching and would treat `te-claude-2` as a hit for `te-claude`).
fn unique_session_name(slug: &str) -> String {
    let base = format!("te-{slug}");
    let existing: std::collections::HashSet<String> = std::process::Command::new("tmux")
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
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

/// Spawn `command` in a NEW, dedicated tmux session (one session per agent, so
/// attaching to one never drags in the others) named `te-<slug>[-N]`, with a
/// single window `window_name` in `abs_path`, tagged with the agent's identity
/// env. Returns the new pane id. Shared by `spawn_agent` and `resume_agent`.
async fn open_agent_session(
    slug: &str,
    window_name: &str,
    abs_path: &str,
    command: &[String],
    group: Option<&str>,
) -> Result<String> {
    let session_name = unique_session_name(slug);

    // Build the new-session command.
    //
    // The spawned agent's slug MUST travel into the pane's environment via tmux's
    // `-e` flag, NOT via `.env()` on the tmux client below: tmux builds a new
    // pane's environment from the server's environment plus `-e` overrides, so a
    // var set only on the client process that issues `new-session` is dropped and
    // never reaches the pane. The session-start hook prefers `TENEX_EDGE_AGENT`
    // over `TENEX_EDGE_AGENT_FALLBACK` and the harness's own default slug (see
    // cli/hooks.rs), so without this the spawn's known identity is lost and a
    // `codex` (or any custom) agent registers under the harness default (e.g.
    // `claude`) — the wrong name in `who`/`tmux`.
    let agent_env = format!("TENEX_EDGE_AGENT={slug}");
    // Forward THIS daemon's tenex-edge home/config/binary into the pane's env via
    // `-e`, for the same reason the slug travels this way: a tmux pane inherits the
    // SERVER's global env, not the daemon's, so without this a spawned harness's
    // hooks phone home to the wrong daemon. The daemon runs with these set (it was
    // started under them); forwarding them makes `tenex-edge launch <agent>` work
    // against a non-default TENEX_EDGE_HOME and pins the exact binary the hooks run.
    let mut passthrough_env: Vec<String> = Vec::new();
    for key in ["TENEX_EDGE_HOME", "TENEX_CONFIG", "TENEX_EDGE_BIN"] {
        if let Ok(val) = std::env::var(key) {
            if !val.is_empty() {
                passthrough_env.push(format!("{key}={val}"));
            }
        }
    }
    // Scope the spawned session to a NIP-29 subgroup task room when requested:
    // the session-start hook forwards TENEX_EDGE_CHANNEL so the daemon stores the
    // session (and publishes its presence/chat) under the child `h` while the
    // working directory stays the parent project's repo.
    if let Some(g) = group.filter(|g| !g.is_empty()) {
        passthrough_env.push(format!("TENEX_EDGE_CHANNEL={g}"));
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
        // Sanitize the parent's Claude Code session identity before exec. The
        // tmux SERVER's global environment can carry CLAUDE_CODE_SESSION_ID /
        // CLAUDE_CODE_CHILD_SESSION (set whenever a `claude` ran under this
        // server), and a new pane inherits it. Left intact, a freshly-spawned
        // `claude` would adopt that foreign id — hijacking another session's
        // transcript instead of starting its own (so its hook-reported id never
        // gets a resumable transcript), and a `--resume <id>` launch would
        // collide with the inherited id. `env -u` strips them; harmless for
        // codex/opencode, which ignore these vars.
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

    // Make tmux transparent to the user. The ONLY reason we run inside tmux is
    // its persistent-session feature; every other tmux behavior (prefix key,
    // mouse capture, ESC delay) gets in the way of the harness and betrays that
    // we're running under tmux. These per-session options drop the prefix
    // entirely, disable mouse capture, kill the ESC delay, and let
    // passthrough/focus sequences reach the app — so the harness appears to own
    // the terminal directly. The status bar is the ONE chrome surface we keep:
    // it's a pure display surface tmux owns at the bottom, driven by
    // `tenex-edge statusline` so every harness (not just claude, which has
    // ccstatusline) gets the fabric awareness floor.
    let tenex_bin = std::env::var("TENEX_EDGE_BIN")
        .ok()
        .filter(|s| !s.is_empty());
    let status_cmd_override = crate::config::Config::load()
        .ok()
        .and_then(|c| c.tmux_status_command);
    make_session_transparent(&session_name, tenex_bin.as_deref(), slug, abs_path, status_cmd_override.as_deref())?;

    Ok(pane_id)
}

/// Apply per-session tmux options that make the session invisible to the user.
/// Called once right after `new-session` so every spawn path — `launch`, the
/// TUI's spawn action, and spawn-on-send — is uniformly transparent.
///
/// `tenex_bin` is the absolute path to forward into the status bar's
/// `tenex-edge statusline` invocation, when known (`TENEX_EDGE_BIN`). When
/// unknown, the bare `tenex-edge` is used and must be on the tmux server's
/// PATH.
fn make_session_transparent(
    session: &str,
    tenex_bin: Option<&str>,
    slug: &str,
    abs_path: &str,
    status_cmd_override: Option<&str>,
) -> Result<()> {
    // Each `(option, value)` pair is applied with `set-option -t <session>`.
    // `-g` is NOT used: we only want to affect this one session, not the global
    // tmux environment (the user may have their own tmux sessions that should
    // keep working normally with their own config).
    //
    // `tenex-edge statusline` fails open (daemon down → empty line, exit 0),
    // so the status bar stays quiet when there's no daemon and never blocks the
    // harness. The status bar is the one piece of tmux chrome we keep: it's a
    // pure display surface that gives every harness (claude, codex, opencode,
    // …) the same fabric awareness floor that ccstatusline currently gives only
    // claude. Refresh at 3s matches ccstatusline's cadence.
    //
    // The `#(...)` status-format command runs in the tmux SERVER's environment,
    // not the pane's — TENEX_EDGE_AGENT and the project cwd are not available
    // there. We store them as tmux user options (@te_agent, @te_cwd) and pass
    // them explicitly via --agent and --cwd so session resolution works.
    // #{q:@te_cwd} applies shell quoting so paths with spaces are handled safely.
    let bin = tenex_bin.unwrap_or("tenex-edge");
    // #{@te_agent} and #{q:@te_cwd} are tmux format variables expanded by tmux
    // before the shell runs the command. #{q:...} adds shell quoting so paths
    // with spaces are safe. In Rust format strings, {{ and }} produce literal
    // { and } respectively.
    let statusline_cmd = status_cmd_override
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            format!("#({bin} statusline --tmux --agent #{{@te_agent}} --cwd #{{q:@te_cwd}})")
        });
    let OPTIONS: Vec<(&str, String)> = vec![
        // Session identity for the status bar: stored as user options so the
        // #(...) status-format command can read them via #{@te_agent} /
        // #{q:@te_cwd} without needing the pane's process environment.
        ("@te_agent", slug.to_string()),
        ("@te_cwd", abs_path.to_string()),
        // Status bar ON — driven by `tenex-edge statusline`, fail-open.
        ("status-style", "default".to_string()),
        ("status", "on".to_string()),
        ("status-interval", "3".to_string()),
        ("status-left", String::new()),
        ("status-right", String::new()),
        // status-format is a tmux array option — setting it by name splits the
        // value on spaces into separate entries, breaking the #(...) command.
        // Use an explicit index to set a single entry as one token.
        ("status-format[0]", statusline_cmd),
        // Drop the prefix key entirely so EVERY keystroke (including Ctrl-b)
        // goes straight to the harness. With `prefix None`, tmux never
        // intercepts Ctrl-b as a prefix, so the user doesn't need to double-tap
        // Ctrl-b to send one. Detach is handled by `tenex-edge tmux` / `launch`,
        // not by Ctrl-b d.
        ("prefix", "None".to_string()),
        // No ESC delay: vim-style apps see ESC immediately.
        ("escape-time", "0".to_string()),
        // Mouse off so wheel/trackpad events pass through to the harness as
        // arrow keys instead of entering tmux copy-mode.
        ("mouse", "off".to_string()),
        // Let apps emit control sequences tmux would otherwise swallow
        // (tmux 3.3+; silently ignored on older versions).
        ("allow-passthrough", "on".to_string()),
        // Apps see focus/blur events.
        ("focus-events", "on".to_string()),
        // True color + extended keys + better key reporting.
        ("default-terminal", "tmux-256color".to_string()),
        ("terminal-overrides", ",*:Tc,RGB,extkeys".to_string()),
    ];

    for (opt, val) in &OPTIONS {
        let status = std::process::Command::new("tmux")
            .args(["set-option", "-t", session, opt, val])
            .status()
            .with_context(|| format!("tmux set-option {opt}"))?;
        // `allow-passthrough` and `terminal-overrides` syntax vary across tmux
        // versions; treat them as best-effort so a legacy tmux doesn't abort
        // an otherwise-fine spawn.
        if !status.success() && !matches!(*opt, "allow-passthrough" | "terminal-overrides") {
            anyhow::bail!("tmux set-option {opt} {val} failed for session {session}");
        }
    }

    Ok(())
}

/// Spawn a new tmux window running `slug`'s harness in `project`'s directory.
/// Returns the new pane id (e.g. "%7") or an error.
///
/// `client_cwd`, when supplied, is the absolute path the client invoked the
/// spawn from; it overrides `project_paths` lookup so the agent lands in the
/// user's actual cwd, not whichever worktree last fired `session_start`.
pub async fn spawn_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
    launch_args: Vec<String>,
    group: Option<&str>,
    client_cwd: Option<&std::path::Path>,
) -> Result<String> {
    if !tmux_available() {
        anyhow::bail!("tmux binary not found");
    }

    let (base_command, agent_def) = resolve_spawn_entry(slug)?;
    let mut agent_command = apply_agent_def_args(base_command, slug, agent_def);
    if !launch_args.is_empty() {
        agent_command.extend(launch_args);
    }
    let def = find_spawn_def(slug); // optional, for the window_name default
    let window_name_owned: String;
    let window_name: &str = match def {
        Some(d) => d.window_name,
        None => {
            window_name_owned = format!("{}·tenex-edge", slug);
            &window_name_owned
        }
    };

    let abs_path = project_abs_path(state, project, client_cwd);
    let pane_id = open_agent_session(slug, window_name, &abs_path, &agent_command, group).await?;

    // No prompt is injected by spawn alone. A spawn-on-send caller tags this
    // pane with its triggering mention via `register_pending_spawn_with_mention`,
    // which is what makes `rpc_session_start` type the message in. A manual spawn
    // tags nothing and so starts clean.
    Ok(pane_id)
}

/// Resume a prior session by replaying its harness with the native resume token.
/// Spawns a NEW tmux window running the agent's configured launch command,
/// transformed into a resume invocation (`claude --resume <id>`, etc.). Unlike
/// `spawn_agent`, NO first prompt is injected — the harness restores its own
/// conversation. When the resumed harness fires `session-start` it re-registers
/// the (same, for claude/codex) session id and a fresh pane endpoint, so the
/// session comes back alive automatically. Returns the new pane id.
pub async fn resume_agent(
    state: &Arc<DaemonState>,
    slug: &str,
    project: &str,
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
    let abs_path = project_abs_path(state, project, None);
    // Re-scope the resumed session to the SAME group it was in. For a subgroup
    // session `project` is the child `h`; passing it as the group override keeps
    // the resumed session publishing into that subgroup. For an ordinary session
    // `project` equals the working-dir project, so this is a harmless no-op.
    open_agent_session(
        slug,
        &window_name,
        &abs_path,
        &resume_command,
        Some(project),
    )
    .await
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

#[cfg(test)]
mod resume_command_tests {
    use super::*;

    fn cmd(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn append_flag_preserves_user_launch_flags() {
        // developer's real config: `claude --dangerously-skip-permissions`.
        let base = cmd(&["claude", "--dangerously-skip-permissions"]);
        let got = build_resume_command(&base, ResumeShape::AppendFlag("--resume"), "abc-123");
        assert_eq!(
            got,
            cmd(&[
                "claude",
                "--dangerously-skip-permissions",
                "--resume",
                "abc-123"
            ])
        );
    }

    #[test]
    fn append_flag_bare_command() {
        let got = build_resume_command(
            &cmd(&["opencode"]),
            ResumeShape::AppendFlag("--session"),
            "ses_x",
        );
        assert_eq!(got, cmd(&["opencode", "--session", "ses_x"]));
    }

    #[test]
    fn subcommand_inserts_after_binary_and_keeps_flags() {
        // codex resume is a subcommand: `codex resume <id> <flags>`.
        let base = cmd(&["codex", "--dangerously-bypass-approvals-and-sandbox"]);
        let got = build_resume_command(&base, ResumeShape::Subcommand("resume"), "uuid-9");
        assert_eq!(
            got,
            cmd(&[
                "codex",
                "resume",
                "uuid-9",
                "--dangerously-bypass-approvals-and-sandbox"
            ])
        );
    }

    #[test]
    fn subcommand_bare_command() {
        let got = build_resume_command(
            &cmd(&["codex"]),
            ResumeShape::Subcommand("resume"),
            "uuid-9",
        );
        assert_eq!(got, cmd(&["codex", "resume", "uuid-9"]));
    }

    #[test]
    fn shape_is_keyed_by_binary_not_slug() {
        // A custom agent slug ("developer") whose binary is claude must resolve
        // via the binary — this is the bug found by actually resuming.
        assert!(matches!(
            resume_shape_for_bin("claude"),
            Some(ResumeShape::AppendFlag("--resume"))
        ));
        assert!(matches!(
            resume_shape_for_bin("codex"),
            Some(ResumeShape::Subcommand("resume"))
        ));
        assert!(matches!(
            resume_shape_for_bin("opencode"),
            Some(ResumeShape::AppendFlag("--session"))
        ));
        // Path basename is matched, not the full path.
        assert!(matches!(
            resume_shape_for_bin("/opt/homebrew/bin/claude"),
            Some(ResumeShape::AppendFlag("--resume"))
        ));
        assert!(resume_shape_for_bin("npx").is_none());
    }

    fn sample_session() -> crate::state::SessionRecord {
        crate::state::SessionRecord {
            session_id: "sess-target".into(),
            agent_slug: "claude".into(),
            agent_pubkey: "pk-target".into(),
            project: "proj".into(),
            host: "host-a".into(),
            child_pid: None,
            watch_pid: None,
            created_at: 1000,
            alive: true,
            rel_cwd: String::new(),
        }
    }

    #[test]
    fn pending_message_prompt_contains_the_actual_message_body() {
        let rec = sample_session();
        let row = crate::state::ChatInboxRow {
            chat_event_id: "abcdef123456".into(),
            target_session: rec.session_id.clone(),
            from_pubkey: "pk-sender".into(),
            from_slug: "codex".into(),
            project: "proj".into(),
            body: "please review the tmux delivery path".into(),
            created_at: 100,
            from_session: "sender-session".into(),
            mentioned_session: rec.session_id.clone(),
        };

        let prompt = render_pending_message_prompt(&[row], &rec, 120).unwrap();

        assert!(prompt.contains("please review the tmux delivery path"));
        assert!(!prompt.contains("tenex-edge inbox"));
    }
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
