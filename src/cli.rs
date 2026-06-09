//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{now_secs, session_short_code, short_id, slugify_host};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use owo_colors::OwoColorize;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(
    name = "tenex-edge",
    about = "Citizenship for your agents: identity + awareness on the Nostr fabric."
)]
pub struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    // session-start / session-end / turn-start / turn-check / turn-end are NOT
    // subcommands. They are hook-driven lifecycle steps invoked only through
    // `hook --type <…>`, which parses the harness's stdin payload and calls the
    // corresponding private fn (session_start_inner / session_end / turn_start /
    // turn_check / turn_end). There is no host-facing way — or need — to invoke
    // them by hand.
    /// Mention another agent or a specific session.
    SendMessage {
        /// session-id (or prefix), agent slug, slug@project, or hex pubkey.
        #[arg(value_name = "RECIPIENT")]
        recipient: Option<String>,
        /// Message body.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        /// session-id (or prefix), agent slug, slug@project, or hex pubkey.
        #[arg(long = "recipient", value_name = "RECIPIENT")]
        recipient_flag: Option<String>,
        /// Message body.
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
        /// My session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
    },
    /// List peers currently visible (with session-id prefixes for targeting).
    Who {
        #[arg(long)]
        project: Option<String>,
        /// Include peers whose heartbeat has stopped (stale).
        #[arg(long)]
        all: bool,
        /// Show agents across all projects (overrides --project / cwd resolution).
        #[arg(long)]
        all_projects: bool,
        /// Keep a full-screen live view open, refreshing automatically.
        #[arg(long)]
        live: bool,
        /// Refresh interval for --live, in milliseconds.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
    /// Manage which agents this computer authorizes (owner-scoped allow/block).
    Acl {
        #[command(subcommand)]
        action: Option<AclAction>,
    },
    /// Stream all fabric activity, colorized.
    Tail {
        #[arg(long)]
        project: Option<String>,
    },
    /// Print + drain pending mentions for a session. Used by the opencode
    /// injection path and as a manual "check my messages" command. (Claude Code
    /// and Codex drain via the `hook --type user-prompt-submit` path instead.)
    Inbox {
        /// Session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
    },
    /// Block until a mention arrives for this session, then print it and exit.
    /// Run with run_in_background=true; the agent is woken when this exits.
    WaitForMention {
        /// Session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
        /// Exit after this many seconds even if no mention arrives (0 = infinite).
        #[arg(long, default_value = "300")]
        timeout: u64,
    },
    /// Manage NIP-29 project groups (list, set description).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Connectivity check: publish a test note to the configured relays and read it back.
    Doctor,
    /// Handle a hook event from any supported agent harness.
    /// Reads hook JSON from stdin; emits context to inject into the model (if any).
    /// Run `tenex-edge hook --host <name> --type <hook-type>`.
    Hook {
        /// Harness name: "claude-code", "codex", … Run `--host help` to list.
        #[arg(long)]
        host: String,
        /// Hook type the harness uses: "session-start", "user-prompt-submit", etc.
        #[arg(long = "type")]
        hook_type: String,
    },
    /// Internal: the per-machine daemon. Spawned automatically; not for direct use.
    /// (Replaces the old detached per-session engine, which now runs as an async
    /// task inside this one daemon — the sole writer of state.db.)
    #[command(name = "__daemon", hide = true)]
    Daemon,
}

#[derive(Subcommand)]
enum AclAction {
    /// List pending (unauthorized) + authorized + blocked agents.
    List,
    /// Authorize an agent (pubkey or pending-list slug) to be seen/trusted.
    Allow { target: String },
    /// Block an agent (pubkey or pending-list slug).
    Block { target: String },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List all NIP-29 project groups on the relay.
    List,
    /// Set the description for a project's NIP-29 group (publishes kind:9002).
    Edit {
        /// New description text.
        #[arg(long)]
        description: String,
        /// Project slug; defaults to the project resolved from the current directory.
        #[arg(long)]
        project: Option<String>,
    },
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::SendMessage {
            recipient,
            message,
            recipient_flag,
            message_flag,
            session,
        } => {
            let recipient = recipient_flag
                .or(recipient)
                .context("missing recipient; use `tenex-edge send-message --recipient <target> --message \"...\"`")?;
            let message = resolve_send_message_body(message_flag.or(message))?;
            send_message(recipient, message, session).await
        }
        Cmd::Who {
            project,
            all,
            all_projects,
            live,
            refresh_ms,
        } => {
            if live {
                who_live(project, all, all_projects, Duration::from_millis(refresh_ms.max(100)))
            } else {
                who(project, all, all_projects)
            }
        }
        Cmd::Acl { action } => acl(action).await,
        Cmd::Tail { project } => tail(project).await,
        Cmd::Inbox { session } => inbox(session).await,
        Cmd::WaitForMention { session, timeout } => wait_for_mention(session, timeout).await,
        Cmd::Project { action } => project(action).await,
        Cmd::Doctor => doctor().await,
        Cmd::Hook { host, hook_type } => hook_run(host, hook_type).await,
        Cmd::Daemon => crate::daemon::server::run().await,
    }
}

/// A peer is "live" only while heartbeats keep it fresh (3× the default 30s tick).
const PEER_FRESH_SECS: u64 = 90;

// Session resolution, session-id generation, recipient resolution, and the
// store live INSIDE the daemon now (it is the sole writer). The CLI verbs below
// are thin clients that forward to it over the UDS.

// ── session-start ────────────────────────────────────────────────────────────

/// Core session-start logic. Returns the resolved session id.
/// Callers decide what to do with it (print for CLI, discard for hooks).
///
/// Thin client: asks the per-machine daemon to spawn an in-process session task
/// (the relocated engine). The daemon is the sole writer of state.db and owns
/// the single relay connection — no more per-session fork.
fn session_start_inner(
    agent: String,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    watch_pid: Option<i32>,
) -> Result<String> {
    let cwd = cwd.unwrap_or(std::env::current_dir()?);
    let params = serde_json::json!({
        "agent": agent,
        "session_id": session_id,
        "cwd": cwd.to_string_lossy(),
        "watch_pid": watch_pid,
    });
    let v = crate::daemon::blocking::call("session_start", params)?;
    let sid = v["session_id"]
        .as_str()
        .context("daemon returned no session_id")?
        .to_string();
    Ok(sid)
}

// ── session-end ──────────────────────────────────────────────────────────────

fn session_end(session: String) -> Result<()> {
    let v = crate::daemon::blocking::call("session_end", serde_json::json!({"session": session}))?;
    if v["ended"].as_bool().unwrap_or(false) {
        eprintln!("session {session} ended");
    } else {
        eprintln!("no such session: {session}");
    }
    Ok(())
}

// ── send-message ─────────────────────────────────────────────────────────────

async fn send_message(recipient: String, message: String, session: Option<String>) -> Result<()> {
    let params = serde_json::json!({
        "recipient": recipient,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("send_message", params).await?;
    let to_pubkey = v["to_pubkey"].as_str().unwrap_or_default().to_string();
    let target_session = v["target_session"].as_str().map(str::to_string);
    match target_session {
        Some(s) => println!("mentioned {} (session {})", short_id(&to_pubkey), short_id(&s)),
        None => println!("mentioned {}", short_id(&to_pubkey)),
    }
    Ok(())
}

/// Async daemon call helper for `async fn` verbs (uses the async client; we are
/// inside the tokio runtime so we must NOT block_on a sync client here).
async fn daemon_call_async(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

fn resolve_send_message_body(raw: Option<String>) -> Result<String> {
    match raw {
        Some(message) if message == "-" => read_stdin_message(),
        Some(message) if message.is_empty() => bail!("message must not be empty"),
        Some(message) => Ok(message),
        None => {
            if io::stdin().is_terminal() {
                bail!(
                    "missing message; use `tenex-edge send-message --recipient <target> --message \"...\"` \
                     or pipe/heredoc the message on stdin"
                );
            }
            read_stdin_message()
        }
    }
}

fn read_stdin_message() -> Result<String> {
    let mut message = String::new();
    io::stdin()
        .read_to_string(&mut message)
        .context("failed to read message from stdin")?;
    let message = strip_single_trailing_newline(message);
    if message.is_empty() {
        bail!("message from stdin was empty");
    }
    Ok(message)
}

fn strip_single_trailing_newline(mut s: String) -> String {
    if s.ends_with('\n') {
        s.pop();
        if s.ends_with('\r') {
            s.pop();
        }
    }
    s
}

// ── who ──────────────────────────────────────────────────────────────────────

/// `who` params for the daemon RPC. The daemon resolves the current project the
/// same way the old CLI did (`all_projects ? None : resolve(cwd)`).
fn who_params(project: &Option<String>, all: bool, all_projects: bool) -> serde_json::Value {
    serde_json::json!({
        "project": project,
        "all": all,
        "all_projects": all_projects,
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    })
}

fn who_snapshot_via_daemon(
    project: &Option<String>,
    all: bool,
    all_projects: bool,
) -> Result<WhoSnapshot> {
    let v = crate::daemon::blocking::call("who", who_params(project, all, all_projects))?;
    Ok(serde_json::from_value(v)?)
}

fn who(project: Option<String>, all: bool, all_projects: bool) -> Result<()> {
    let snapshot = who_snapshot_via_daemon(&project, all, all_projects)?;
    print!("{}", render_who_once(&snapshot));
    Ok(())
}

fn who_live(project: Option<String>, all: bool, all_projects: bool, refresh: Duration) -> Result<()> {
    let refresh = refresh.max(Duration::from_millis(100));
    let _terminal = LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let snapshot = who_snapshot_via_daemon(&project, all, all_projects)?;
            draw_who_live(&snapshot, refresh)?;
            next_draw = Instant::now() + refresh;
        }

        let wait = next_draw
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100));
        if event::poll(wait)? {
            if should_quit_live(event::read()?) {
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OtherProjectSummary {
    project: String,
    agent_count: usize,
    #[serde(default)]
    agents: Vec<String>,
    about: Option<String>,
}

// The daemon serializes a WhoSnapshot and the thin `who` client renders it with
// the EXACT renderers below — so output is byte-identical by construction and
// can never drift from a separate copy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WhoSnapshot {
    project: String,
    all: bool,
    now: u64,
    rows: Vec<WhoRow>,
    other_projects: Vec<OtherProjectSummary>,
}


#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct WhoRow {
    source: WhoSource,
    fresh: bool,
    slug: String,
    project: String,
    status: String,
    host: String,
    session_id: String,
    age_secs: Option<u64>,
    /// Project-relative working dir (§8e). Empty or "." → rendered without a
    /// `[dir]` bracket; otherwise shown so worktrees render distinctly.
    #[serde(default)]
    rel_cwd: String,
    /// True only for a peer whose host differs from the daemon/viewer's host.
    /// Local sessions and same-machine peers are never remote (the §8e fix).
    #[serde(default)]
    remote: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum WhoSource {
    Local,
    Peer,
}

pub fn load_who_snapshot(
    store: &Store,
    current_project: Option<&str>,
    all: bool,
    now: u64,
    daemon_host: &str,
) -> Result<WhoSnapshot> {
    // §8e: "remote" is computed DAEMON-side by comparing each peer's host to the
    // daemon's own host, so all rendering stays client-side and can't diverge via
    // a second Config::load(). Local sessions are on this machine by construction
    // → never remote. A peer is remote ONLY when its host differs from ours.
    let local_host = slugify_host(daemon_host);
    let since = if all { 0 } else { now.saturating_sub(PEER_FRESH_SECS) };

    let mine = store.list_my_live_sessions(since)?;
    let my_ids: std::collections::HashSet<String> =
        mine.iter().map(|s| s.session_id.clone()).collect();
    let all_peers: Vec<_> = store
        .list_peer_sessions(None, since)?
        .into_iter()
        .filter(|p| !my_ids.contains(&p.session_id))
        .collect();

    let mut rows = Vec::new();
    let mut other_agents: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();

    for s in &mine {
        let age_secs = store
            .session_last_seen(&s.session_id)
            .ok()
            .flatten()
            .map(|ls| now.saturating_sub(ls));
        if current_project.map(|p| p == s.project).unwrap_or(true) {
            rows.push(WhoRow {
                source: WhoSource::Local,
                fresh: age_secs.map(|a| a <= PEER_FRESH_SECS).unwrap_or(true),
                slug: s.agent_slug.clone(),
                project: s.project.clone(),
                status: status_for(store, &s.agent_pubkey, &s.project),
                host: s.host.clone(),
                session_id: s.session_id.clone(),
                age_secs,
                rel_cwd: s.rel_cwd.clone(),
                remote: false,
            });
        } else {
            other_agents
                .entry(s.project.clone())
                .or_default()
                .insert(s.agent_slug.clone());
        }
    }

    for p in &all_peers {
        let age = now.saturating_sub(p.last_seen);
        if current_project.map(|cp| cp == p.project).unwrap_or(true) {
            rows.push(WhoRow {
                source: WhoSource::Peer,
                fresh: age <= PEER_FRESH_SECS,
                slug: p.slug.clone(),
                project: p.project.clone(),
                status: status_for(store, &p.pubkey, &p.project),
                host: p.host.clone(),
                session_id: p.session_id.clone(),
                age_secs: Some(age),
                rel_cwd: p.rel_cwd.clone(),
                remote: slugify_host(&p.host) != local_host,
            });
        } else {
            other_agents
                .entry(p.project.clone())
                .or_default()
                .insert(p.slug.clone());
        }
    }

    let other_projects = other_agents
        .into_iter()
        .map(|(project, agents)| {
            let about = store.get_project_meta(&project).ok().flatten();
            let agents: Vec<String> = agents.into_iter().collect();
            OtherProjectSummary {
                project,
                agent_count: agents.len(),
                agents,
                about,
            }
        })
        .collect();

    Ok(WhoSnapshot {
        project: current_project.unwrap_or("*").to_string(),
        all,
        now,
        rows,
        other_projects,
    })
}

fn status_for(store: &Store, pubkey: &str, project: &str) -> String {
    store
        .get_agent_status(pubkey, project)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Append the turn-start "tenex-edge fabric" block(s): the full roster on the
/// first turn, or "changes since your last turn" afterward. This is the single
/// source of truth — both the CLI `turn_start` and the daemon's `turn_start` RPC
/// call it, so the injected text is identical.
pub fn push_turn_fabric_block(
    store: &std::sync::Mutex<Store>,
    blocks: &mut Vec<String>,
    first_turn: bool,
    prev_turn_started_at: u64,
    project: &str,
    now: u64,
    daemon_host: &str,
) {
    let store = store.lock().expect("store mutex poisoned");
    if first_turn {
        if let Ok(snapshot) = load_who_snapshot(&store, Some(project), false, now, daemon_host) {
            if !snapshot.rows.is_empty() {
                let who_text = render_who_plain(&snapshot);
                blocks.push(format!(
                "tenex-edge fabric — agents you can message. To send, run \
                 `tenex-edge send-message --recipient <agent@project|session-id> --message \"...\"`:\n{}",
                who_text.trim_end()
            ));
        }
        }
    } else {
        let fresh_since = now.saturating_sub(PEER_FRESH_SECS);
        let new_peers = store
            .list_new_peer_sessions(prev_turn_started_at, fresh_since, Some(project))
            .unwrap_or_default();
        let status_changes = store
            .list_status_changes_since(prev_turn_started_at, Some(project))
            .unwrap_or_default();

        let mut delta: Vec<String> = Vec::new();
        for p in &new_peers {
            let age = now.saturating_sub(p.last_seen);
            delta.push(format!(
                "  ● {}@{} joined  {}  session {}  ({age}s ago)",
                p.slug,
                slugify_host(&p.host),
                p.project,
                short_id(&p.session_id),
            ));
        }
        for (slug, proj, text) in &status_changes {
            delta.push(format!("  ↻ {slug}@{proj} — {text}"));
        }
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes since your last turn:\n{}",
                delta.join("\n")
            ));
        }
    }
}

fn render_who_once(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();

    let scope = if snapshot.project == "*" {
        "all projects".to_string()
    } else {
        snapshot.project.clone()
    };
    let _ = writeln!(out, "{}", scope.bold());
    let _ = writeln!(out);

    if snapshot.rows.is_empty() {
        let _ = writeln!(
            out,
            "(no live agents{})",
            if snapshot.all {
                ""
            } else {
                " — start a session, or run with --all to include stale"
            }
        );
    } else if snapshot.project == "*" {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, true);
        }
    } else {
        for row in &snapshot.rows {
            render_who_row(&mut out, row, false);
        }
    }

    if snapshot.project != "*" && !snapshot.other_projects.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "{} other agent(s) in other projects:", snapshot.other_projects.len());
        for op in &snapshot.other_projects {
            match &op.about {
                Some(about) if !about.is_empty() => {
                    let _ = writeln!(out, "  * {} - {}", op.project, about);
                }
                _ => {
                    let _ = writeln!(out, "  * {}", op.project);
                }
            }
        }
    }

    out
}

fn render_who_row(out: &mut String, row: &WhoRow, include_project: bool) {
    let stale = if row.fresh {
        String::new()
    } else {
        format!(" {}", "(stale)".dimmed())
    };
    // §8e: same-machine agents get NO annotation; a true remote (peer on
    // a different host than the daemon) gets ` (remote)`.
    let remote = if row.remote {
        format!(" {}", "(remote)".dimmed())
    } else {
        String::new()
    };
    let dir = rel_cwd_bracket(&row.rel_cwd)
        .map(|d| format!(" {}", format!("[{d}]").dimmed()))
        .unwrap_or_default();
    let name = if include_project {
        format!("{}@{}", row.slug, row.project).cyan().to_string()
    } else {
        row.slug.cyan().to_string()
    };
    let _ = writeln!(
        out,
        "{} [session {}]{}{}{} - {}",
        name,
        session_short_code(&row.session_id).yellow(),
        dir,
        remote,
        stale,
        status_plain(&row.status),
    );
}

/// The `[dir]` to show for a row's `rel_cwd`: `None` when empty or the project
/// root (`.`), so the project root renders without a bracket (§8e).
fn rel_cwd_bracket(rel_cwd: &str) -> Option<&str> {
    let r = rel_cwd.trim();
    if r.is_empty() || r == "." {
        None
    } else {
        Some(r)
    }
}

fn draw_who_live(snapshot: &WhoSnapshot, refresh: Duration) -> Result<()> {
    let refresh_ms = refresh.as_millis();
    let mut screen = render_who_once(snapshot);
    let _ = writeln!(
        screen,
        "{}",
        format!("  --live  refresh {refresh_ms}ms  q/esc/ctrl-c to quit").dimmed()
    );
    let mut stdout = io::stdout();
    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;
    for line in screen.lines() {
        write!(stdout, "{line}\r\n")?;
    }
    stdout.flush()?;
    Ok(())
}

fn status_plain(status: &str) -> String {
    if status.trim().is_empty() {
        "idle".to_string()
    } else {
        status.trim().to_string()
    }
}


fn should_quit_live(event: TermEvent) -> bool {
    let TermEvent::Key(key) = event else {
        return false;
    };
    matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
        || (matches!(key.code, KeyCode::Char('c')) && key.modifiers.contains(KeyModifiers::CONTROL))
}

struct LiveTerminal;

impl LiveTerminal {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for LiveTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}

#[cfg(test)]
mod who_tests {
    use super::*;

    fn strip_ansi(input: &str) -> String {
        let mut out = String::new();
        let mut chars = input.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    fn local_session(id: &str) -> crate::state::SessionRecord {
        crate::state::SessionRecord {
            session_id: id.to_string(),
            agent_slug: "coder".to_string(),
            agent_pubkey: "pk-coder".to_string(),
            project: "proj".to_string(),
            host: "laptop".to_string(),
            child_pid: Some(42),
            watch_pid: None,
            created_at: 1,
            alive: true,
            rel_cwd: String::new(),
        }
    }

    #[test]
    fn who_snapshot_merges_local_and_peer_sessions() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_session(&local_session("local-session"))
            .unwrap();
        store.touch_session("local-session", 1_000).unwrap();
        store
            .upsert_peer_session(
                "local-session",
                "pk-coder",
                "coder",
                "proj",
                "laptop",
                "",
                1_000,
            )
            .unwrap();
        store
            .upsert_peer_session(
                "remote-session",
                "pk-reviewer",
                "reviewer",
                "proj",
                "tower",
                "",
                995,
            )
            .unwrap();
        store
            .set_agent_status("pk-reviewer", "proj", "reviewing the patch", 995)
            .unwrap();

        // Daemon/viewer host is "laptop" → the local coder is same-machine; the
        // "tower" reviewer is a genuine remote.
        let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();

        assert_eq!(snapshot.rows.len(), 2);
        let coder = snapshot
            .rows
            .iter()
            .find(|r| r.source == WhoSource::Local && r.slug == "coder")
            .expect("local coder row");
        let reviewer = snapshot
            .rows
            .iter()
            .find(|r| r.source == WhoSource::Peer && r.slug == "reviewer")
            .expect("peer reviewer row");
        assert!(!snapshot
            .rows
            .iter()
            .any(|r| r.source == WhoSource::Peer && r.session_id == "local-session"));

        // §8e same-host/remote: this machine's own session is NOT remote; a peer
        // on a different host IS.
        assert!(!coder.remote, "local session must never be remote");
        assert!(reviewer.remote, "tower peer must be remote vs laptop");

        let once = strip_ansi(&render_who_once(&snapshot));
        assert!(once.starts_with("proj\n\n"));
        assert!(once.contains("coder [session local-se] - idle"));
        assert!(once.contains("coder"));
        // The remote tag appears only for the genuine remote.
        assert!(once.contains("(remote)"));
    }

    #[test]
    fn same_host_peer_is_not_remote() {
        // A sibling agent (e.g. codex@) on the SAME laptop arrives as a peer row;
        // it must NOT be tagged remote (the bug being fixed).
        let store = Store::open_memory().unwrap();
        store
            .upsert_peer_session("sib", "pk-codex", "codex", "proj", "laptop", "worktree1", 1_000)
            .unwrap();
        let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
        let sib = snap.rows.iter().find(|r| r.slug == "codex").expect("sibling row");
        assert!(!sib.remote, "same-host peer must not be remote");
        assert_eq!(sib.rel_cwd, "worktree1");
        let once = strip_ansi(&render_who_once(&snap));
        assert!(!once.contains("(remote)"), "no remote tag for same-host peer");
        assert!(once.contains("[worktree1]"), "rel_cwd shown in bracket");
    }

    #[test]
    fn root_rel_cwd_has_no_bracket() {
        let store = Store::open_memory().unwrap();
        // rel_cwd "." (project root) → no [dir] bracket.
        store
            .upsert_peer_session("r", "pk-a", "a", "proj", "tower", ".", 1_000)
            .unwrap();
        let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
        let once = strip_ansi(&render_who_once(&snap));
        assert!(!once.contains("[.]"), "root cwd must not render a bracket");
        assert!(once.contains("(remote)"));
    }

    #[test]
    fn live_renderer_same_as_once_with_hint() {
        let snapshot = WhoSnapshot {
            project: "proj".to_string(),
            all: false,
            now: 1_000,
            rows: vec![WhoRow {
                source: WhoSource::Peer,
                fresh: true,
                slug: "reviewer".to_string(),
                project: "proj".to_string(),
                status: "reviewing the patch".to_string(),
                host: "tower".to_string(),
                session_id: "remote-session".to_string(),
                age_secs: Some(5),
                rel_cwd: String::new(),
                remote: false,
            }],
            other_projects: vec![],
        };

        // --live uses render_who_once: same content, plus a dim quit-hint footer.
        let once = strip_ansi(&render_who_once(&snapshot));
        assert!(once.contains("reviewer"));
        assert!(once.contains("reviewing the patch"));
    }

    #[test]
    fn who_renderer_summarizes_other_projects() {
        let store = Store::open_memory().unwrap();
        store
            .upsert_peer_session("s1", "pk-a", "a", "proj", "laptop", "", 1_000)
            .unwrap();
        store
            .upsert_peer_session("s2", "pk-b", "b", "other", "laptop", "", 1_000)
            .unwrap();
        store
            .upsert_peer_session("s3", "pk-b", "b", "other", "laptop", "worktree", 1_001)
            .unwrap();
        store.upsert_project_meta("other", "Other work", 1_000).unwrap();

        let snap = load_who_snapshot(&store, Some("proj"), false, 1_000, "laptop").unwrap();
        let once = strip_ansi(&render_who_once(&snap));

        assert!(once.contains("a [session s1] - idle"));
        assert!(once.contains("1 other agent(s) in other projects:"));
        assert!(once.contains("  * other - Other work"));
    }

    #[test]
    fn who_all_projects_includes_project_in_agent_names() {
        let snapshot = WhoSnapshot {
            project: "*".to_string(),
            all: false,
            now: 1_000,
            rows: vec![WhoRow {
                source: WhoSource::Peer,
                fresh: true,
                slug: "reviewer".to_string(),
                project: "other".to_string(),
                status: String::new(),
                host: "tower".to_string(),
                session_id: "remote-session".to_string(),
                age_secs: Some(5),
                rel_cwd: String::new(),
                remote: false,
            }],
            other_projects: vec![],
        };

        let once = strip_ansi(&render_who_once(&snapshot));
        assert!(once.starts_with("all projects\n\n"));
        assert!(once.contains("reviewer@other [session remote-s] - idle"));
    }

}

// ── acl (owner-scoped agent authorization) ───────────────────────────────────

async fn acl(action: Option<AclAction>) -> Result<()> {
    match action {
        Some(AclAction::Allow { target }) => {
            let v = daemon_call_async("acl", serde_json::json!({"action": "allow", "target": target})).await?;
            println!(
                "authorized {} ({})",
                v["slug"].as_str().unwrap_or("").cyan(),
                short_id(v["pubkey"].as_str().unwrap_or(""))
            );
        }
        Some(AclAction::Block { target }) => {
            let v = daemon_call_async("acl", serde_json::json!({"action": "block", "target": target})).await?;
            println!(
                "blocked {} ({})",
                v["slug"].as_str().unwrap_or(""),
                short_id(v["pubkey"].as_str().unwrap_or(""))
            );
        }
        Some(AclAction::List) | None => {
            let v = daemon_call_async("acl", serde_json::json!({"action": "list"})).await?;
            println!(
                "{}",
                "pending (claim you as owner, awaiting your decision):".bold()
            );
            let pending = v["pending"].as_array().cloned().unwrap_or_default();
            if pending.is_empty() {
                println!("  (none)");
            } else {
                for p in &pending {
                    println!(
                        "  {} {}  ({})  host {}",
                        "?".yellow(),
                        p["slug"].as_str().unwrap_or("").cyan(),
                        short_id(p["pubkey"].as_str().unwrap_or("")),
                        p["host"].as_str().unwrap_or("").dimmed()
                    );
                }
                println!(
                    "\n  allow:  tenex-edge acl allow <slug|pubkey>\n  block:  tenex-edge acl block <slug|pubkey>"
                );
            }
            println!(
                "\n{} {} authorized, {} blocked",
                "acl:".bold(),
                v["allowed"].as_u64().unwrap_or(0),
                v["blocked"].as_u64().unwrap_or(0)
            );
        }
    }
    Ok(())
}

// ── mention rendering (one place; reused by inbox / wait / turn injection) ────

/// The fully-qualified `--recipient` handle the receiver should reply to. Prefer
/// the sender's exact session id — so a reply reaches the precise sibling session
/// that wrote this — but only when that session actually resolves on our side;
/// otherwise fall back to `slug@project`, which always routes to the agent.
pub fn mention_reply_handle(store: &Store, row: &crate::state::InboxRow) -> String {
    if !row.from_session.is_empty() {
        let resolves = store
            .find_peer_session_by_prefix(&row.from_session)
            .ok()
            .flatten()
            .is_some()
            || store
                .find_session_by_prefix(&row.from_session)
                .ok()
                .flatten()
                .is_some();
        if resolves {
            return row.from_session.clone();
        }
    }
    format!("{}@{}", row.from_slug, row.project)
}

/// One injected line for an inbound mention. `reply_to` is the literal value to
/// pass to `tenex-edge send-message --recipient <reply_to>`.
pub fn format_mention_line(from_slug: &str, project: &str, reply_to: &str, body: &str) -> String {
    format!("[mention from {from_slug}@{project} · reply-to {reply_to}] {body}")
}

// ── inbox ────────────────────────────────────────────────────────────────────

async fn inbox(session: Option<String>) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = daemon_call_async("inbox", params).await?;
    if let Some(rows) = v["rows"].as_array() {
        for r in rows {
            println!(
                "{}",
                format_mention_line(
                    r["from_slug"].as_str().unwrap_or(""),
                    r["project"].as_str().unwrap_or(""),
                    r["reply_to"].as_str().unwrap_or(""),
                    r["body"].as_str().unwrap_or(""),
                )
            );
        }
    }
    if let Some(pending) = v["pending_agents"].as_array().filter(|p| !p.is_empty()) {
        let names: Vec<String> = pending
            .iter()
            .map(|p| {
                format!(
                    "{} ({})",
                    p["slug"].as_str().unwrap_or(""),
                    short_id(p["pubkey"].as_str().unwrap_or(""))
                )
            })
            .collect();
        println!(
            "[tenex-edge] {} unauthorized agent(s) claim your owner: {}. \
They are NOT visible until you decide — tell your human to run `tenex-edge acl` to allow or block them.",
            pending.len(),
            names.join(", ")
        );
    }
    Ok(())
}

// ── wait-for-mention ─────────────────────────────────────────────────────────

async fn wait_for_mention(session: Option<String>, timeout: u64) -> Result<()> {
    // The daemon long-polls: it holds the request open until a mention for this
    // session arrives or the timeout fires, then returns the rows.
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "timeout": timeout,
    });
    let v = daemon_call_async("wait_for_mention", params).await?;
    if let Some(rows) = v["rows"].as_array().filter(|r| !r.is_empty()) {
        for r in rows {
            println!(
                "{}",
                format_mention_line(
                    r["from_slug"].as_str().unwrap_or(""),
                    r["project"].as_str().unwrap_or(""),
                    r["reply_to"].as_str().unwrap_or(""),
                    r["body"].as_str().unwrap_or(""),
                )
            );
        }
        println!("[tenex-edge] Run `tenex-edge wait-for-mention` with run_in_background=true to receive the next mention.");
    }
    Ok(())
}

// ── turn-start / turn-check / turn-end ───────────────────────────────────────

async fn turn_start(session: String, transcript: Option<String>, json_out: bool) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    let params = serde_json::json!({
        "session": session,
        "transcript": transcript,
    });
    let v = daemon_call_async("turn_start", params).await?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, json_out);
    }
    Ok(())
}

/// The full turn-start context assembly, shared by the daemon's `turn_start` RPC
/// (the only caller now). Mutating reads (mark_turn_start, drain, set_transcript)
/// happen here under the shared store; the relay self-fetch is done by the
/// caller beforehand. Single source of truth → injected text cannot drift.
///
/// `prev_turn_started_at` is the turn_state value BEFORE this turn's mark; the
/// caller passes it so first-turn detection matches the old behavior.
pub fn assemble_turn_start_context(
    store: &std::sync::Mutex<Store>,
    rec: &crate::state::SessionRecord,
    prev_turn_started_at: u64,
) -> Option<String> {
    let first_turn = prev_turn_started_at == 0;
    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        blocks.push(
            "[tenex-edge] You are connected to the tenex-edge agent fabric. \
             You can run `tenex-edge who`, `tenex-edge inbox`, and \
             `tenex-edge send-message --recipient <agent@project|session-id> --message \"...\"`. \
             If the user asks you to message/contact/tell another agent, run `tenex-edge send-message`; \
             do not say you cannot send messages from here. Run `tenex-edge wait-for-mention` \
             with run_in_background=true so you are woken when a mention arrives. \
             Re-run it each time one is received."
                .to_string(),
        );
    }

    // Drain inbox (authoritative delivery; turn_check only peeks).
    let inbox_lines = {
        let s = store.lock().expect("store mutex poisoned");
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        // Render each line (incl. its reply-to handle) while we hold the lock —
        // the handle resolution needs the store.
        rows.iter()
            .map(|r| {
                s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                    .ok();
                let handle = mention_reply_handle(&s, r);
                format_mention_line(&r.from_slug, &r.project, &handle, &r.body)
            })
            .collect::<Vec<_>>()
    };
    if !inbox_lines.is_empty() {
        let mut text = String::from("Messages from other agents (tenex-edge):");
        for line in &inbox_lines {
            let _ = write!(text, "\n{line}");
        }
        blocks.push(text);
    }

    // Pending ACL agents (unknown agents claiming this owner).
    let pending = {
        let s = store.lock().expect("store mutex poisoned");
        s.list_pending_agents().unwrap_or_default()
    };
    if !pending.is_empty() {
        let names: Vec<String> = pending
            .iter()
            .map(|p| format!("{} ({})", p.slug, short_id(&p.pubkey)))
            .collect();
        blocks.push(format!(
            "[tenex-edge] {} unauthorized agent(s) claim your owner: {}. \
             They are NOT visible until you decide — tell your human to run \
             `tenex-edge acl` to allow or block them.",
            pending.len(),
            names.join(", ")
        ));
    }

    // Peer presence — full roster on the first turn; deltas on subsequent turns.
    push_turn_fabric_block(store, &mut blocks, first_turn, prev_turn_started_at, &rec.project, now_secs(), &rec.host);

    if blocks.is_empty() {
        None
    } else {
        Some(blocks.join("\n\n"))
    }
}

/// Mid-turn inbox PEEK (read-only) shared by the daemon's `turn_check` RPC.
pub fn assemble_turn_check_context(store: &std::sync::Mutex<Store>, session_id: &str) -> Option<String> {
    let lines = {
        let s = store.lock().expect("store mutex poisoned");
        let rows = s.peek_inbox(session_id).unwrap_or_default();
        rows.iter()
            .map(|r| {
                let handle = mention_reply_handle(&s, r);
                format_mention_line(&r.from_slug, &r.project, &handle, &r.body)
            })
            .collect::<Vec<_>>()
    };
    if lines.is_empty() {
        return None;
    }
    let mut text = String::from("[tenex-edge] Message(s) arrived while you were working:");
    for line in &lines {
        let _ = write!(text, "\n{line}");
    }
    Some(text)
}

/// Mid-turn inbox check for PostToolUse hooks. Thin client: the daemon peeks.
fn turn_check(session: Option<String>, json_out: bool) -> Result<()> {
    let params = serde_json::json!({
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
    });
    let v = crate::daemon::blocking::call("turn_check", params)?;
    if let Some(ctx) = v["context"].as_str() {
        emit_context(ctx, json_out);
    }
    Ok(())
}

fn render_who_plain(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "agents:");
    for row in &snapshot.rows {
        let stale = if row.fresh { "" } else { " (stale)" };
        let remote = if row.remote { " (remote)" } else { "" };
        let dir = rel_cwd_bracket(&row.rel_cwd)
            .map(|d| format!(" [{d}]"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {}@{} [session {}]{}{}{}",
            row.slug,
            row.project,
            short_id(&row.session_id),
            dir,
            remote,
            stale,
        );
        let _ = writeln!(out, "      {}", status_plain(&row.status));
    }
    out
}

fn emit_context(content: &str, json_out: bool) {
    if json_out {
        let obj = serde_json::json!({"systemMessage": content});
        println!("{obj}");
    } else {
        println!("{content}");
    }
}

fn turn_end(session: String) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    crate::daemon::blocking::call("turn_end", serde_json::json!({"session": session}))?;
    Ok(())
}

// ── project ──────────────────────────────────────────────────────────────────

async fn project(action: ProjectAction) -> Result<()> {
    match action {
        ProjectAction::List => {
            let v = daemon_call_async("project_list", serde_json::json!({})).await?;
            let projects = v["projects"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
            if projects.is_empty() {
                println!("No NIP-29 groups found on the relay.");
                return Ok(());
            }
            let max_slug = projects
                .iter()
                .filter_map(|p| p["slug"].as_str())
                .map(|s| s.len())
                .max()
                .unwrap_or(0);
            for p in projects {
                let slug = p["slug"].as_str().unwrap_or("");
                let about = p["about"].as_str().unwrap_or("");
                if about.is_empty() {
                    println!("{slug}");
                } else {
                    println!("{slug:<max_slug$}  — {about}");
                }
            }
        }
        ProjectAction::Edit { description, project } => {
            let slug = project.unwrap_or_else(|| {
                crate::project::resolve(&std::env::current_dir().unwrap_or_default())
            });
            let v = daemon_call_async(
                "project_edit",
                serde_json::json!({ "project": slug, "description": description }),
            )
            .await?;
            let event_id = v["event_id"].as_str().unwrap_or("?");
            println!("Updated {slug}: {}", &event_id[..event_id.len().min(8)]);
        }
    }
    Ok(())
}

// ── doctor ───────────────────────────────────────────────────────────────────

async fn doctor() -> Result<()> {
    // The daemon owns the single relay connection, so it performs the probe.
    let v = daemon_call_async("doctor", serde_json::json!({})).await?;
    if let Some(relays) = v["relays"].as_array() {
        let relays: Vec<&str> = relays.iter().filter_map(|r| r.as_str()).collect();
        println!("relays: {relays:?}");
    }
    if let Some(pk) = v["probe_pubkey"].as_str() {
        println!("probe pubkey: {pk}");
    }
    println!("publish: {}", v["publish"].as_str().unwrap_or("?"));
    println!("read-back: {}", v["readback"].as_str().unwrap_or("?"));
    Ok(())
}

// ── tail ─────────────────────────────────────────────────────────────────────

async fn tail(project: Option<String>) -> Result<()> {
    // The daemon owns the single relay connection and streams decoded, rendered
    // fabric lines over the UDS until we disconnect (Ctrl-C). The rendering uses
    // the SAME `render()` daemon-side, so output is identical.
    let scope_label = project.as_deref().unwrap_or("*");
    eprintln!(
        "{} tailing project {} … (Ctrl-C to stop)",
        "tenex-edge".bold(),
        scope_label.cyan()
    );

    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let stream = client.stream(
        "tail",
        serde_json::json!({ "project": project }),
        |item| {
            if let Some(line) = item.get("line").and_then(|l| l.as_str()) {
                println!("{line}");
            }
        },
    );
    tokio::select! {
        _ = tokio::signal::ctrl_c() => Ok(()),
        r = stream => r,
    }
}

/// Public alias so the daemon's `tail` RPC can render fabric lines identically
/// to the old in-process `tail`.
pub fn render_fabric(de: &DomainEvent) -> String {
    render(de)
}

fn render(de: &DomainEvent) -> String {
    match de {
        DomainEvent::Profile(p) => {
            format!(
                "{} {}@{}",
                "id  ".dimmed(),
                p.agent.slug.cyan(),
                p.host.dimmed()
            )
        }
        DomainEvent::Presence(p) => format!(
            "{} {}@{} {} ({})",
            "live".green(),
            p.agent.slug.cyan(),
            slugify_host(&p.host),
            short_id(&p.session_id).yellow(),
            p.project.dimmed()
        ),
        DomainEvent::Activity(a) => {
            format!("{} {}: {}", "act ".blue(), a.agent.slug.cyan(), a.text)
        }
        DomainEvent::Status(s) if s.is_idle() => {
            format!("{} {} idle", "stat".dimmed(), s.agent.slug.cyan())
        }
        DomainEvent::Status(s) => {
            format!("{} {}: {}", "stat".magenta(), s.agent.slug.cyan(), s.text)
        }
        DomainEvent::Mention(m) => format!(
            "{} {} -> {}{}: {}",
            "msg ".yellow(),
            m.from.slug.cyan(),
            short_id(&m.to_pubkey),
            m.target_session
                .as_deref()
                .map(|s| format!(" ({})", short_id(s)))
                .unwrap_or_default(),
            m.body
        ),
    }
}

// ── hook adapter registry ─────────────────────────────────────────────────────
//
// Adding a new agent harness: add one entry to HOOK_HOSTS. Zero new code needed
// for harnesses that follow the standard pattern (JSON stdin, plain/JSON stdout).
// Non-standard needs (custom PID detection, exotic output formats) extend the
// HostDef fields rather than adding branches to hook_run.

/// How context blocks are returned to the model by a given harness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HookOutputFormat {
    /// Plain text on stdout — Claude Code UserPromptSubmit and most harnesses.
    PlainText,
    /// Codex-style JSON: {"systemMessage": "<content>"} — all Codex hook types.
    JsonSystemMessage,
}

struct HostDef {
    /// Canonical harness name used in --host.
    name: &'static str,
    /// Default agent slug (overridden by TENEX_EDGE_AGENT env var).
    agent_slug: &'static str,
    /// JSON fields tried in order to extract the session id from stdin.
    session_id_fields: &'static [&'static str],
    /// JSON field for the live transcript path (None if the harness omits it).
    transcript_field: Option<&'static str>,
    /// Output format for context injection hooks.
    output_format: HookOutputFormat,
    /// Walk process tree for an ancestor whose command contains this string.
    /// None = no watch-pid. Used by harnesses (e.g. Codex) that omit their PID.
    pid_search: Option<&'static str>,
    /// When true, a session-start payload with no session id makes the daemon
    /// GENERATE one and the hook prints it to stdout — for programmatic hosts
    /// (e.g. opencode) that have no harness-assigned id and capture it back.
    /// When false (Claude Code, Codex), an empty session id is a fail-open
    /// no-op: those harnesses always supply their own id, so a missing one means
    /// a malformed payload, and generating would spawn an orphan session that
    /// later turn-start/stop calls could never match.
    generates_sid: bool,
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        generates_sid: false,
    },
    HostDef {
        name: "codex",
        agent_slug: "codex",
        session_id_fields: &[
            "session_id", "sessionId",
            "conversation_id", "conversationId",
            "thread_id", "threadId",
        ],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::JsonSystemMessage,
        pid_search: Some("codex"),
        generates_sid: false,
    },
    HostDef {
        // opencode is a programmatic TS plugin, not a stdin-JSON harness in the
        // usual sense: it pipes a small JSON payload to `hook` and reads stdout.
        // It has no harness-assigned session id, so session-start generates one
        // and returns it; it passes its own PID in the payload (no pid_search).
        name: "opencode",
        agent_slug: "opencode",
        session_id_fields: &["session_id"],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
        generates_sid: true,
    },
];

fn find_hook_host(name: &str) -> Option<&'static HostDef> {
    if name == "help" {
        eprintln!(
            "known hosts: {}",
            HOOK_HOSTS.iter().map(|h| h.name).collect::<Vec<_>>().join(", ")
        );
        return None;
    }
    HOOK_HOSTS.iter().find(|h| h.name == name)
}

// ── hook_run ──────────────────────────────────────────────────────────────────

async fn hook_run(host_name: String, hook_type: String) -> Result<()> {
    use std::io::Read as _;

    let Some(host) = find_hook_host(&host_name) else {
        eprintln!("[tenex-edge] unknown host {host_name:?}; run `--host help` to list");
        return Ok(());
    };

    let json_out = host.output_format == HookOutputFormat::JsonSystemMessage;
    let agent_slug = std::env::var("TENEX_EDGE_AGENT")
        .unwrap_or_else(|_| host.agent_slug.to_string());

    // Parse stdin — fail open if JSON is absent or malformed.
    let raw: serde_json::Value = {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).ok();
        serde_json::from_str(&buf).unwrap_or(serde_json::Value::Null)
    };
    let obj = raw.as_object();

    let sid: String = host
        .session_id_fields
        .iter()
        .find_map(|f| {
            obj.and_then(|o| o.get(*f))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let cwd: PathBuf = obj
        .and_then(|o| o.get("cwd"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let transcript: Option<String> = host.transcript_field.and_then(|field| {
        obj.and_then(|o| o.get(field))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    });

    match hook_type.as_str() {
        "session-start" => {
            // PID to watch: an explicit `pid`/`watch_pid` in the payload (set by
            // programmatic hosts like opencode, which know their own process)
            // wins; otherwise walk the process tree for the harness's ancestor.
            let watch_pid = obj
                .and_then(|o| o.get("pid").or_else(|| o.get("watch_pid")))
                .and_then(|v| v.as_i64())
                .map(|n| n as i32)
                .or_else(|| host.pid_search.and_then(find_ancestor_pid));

            if sid.is_empty() {
                if !host.generates_sid {
                    // Fail open: a harness that assigns its own id but dropped it
                    // here sent a malformed payload — don't spawn an orphan.
                    return Ok(());
                }
                // Programmatic host with no id of its own: generate one and hand
                // it back on stdout so the caller can adopt it.
                let new_sid = session_start_inner(agent_slug, None, Some(cwd), watch_pid)?;
                println!("{new_sid}");
            } else {
                // Harness supplied its own id — adopt it, discard the echo.
                session_start_inner(agent_slug, Some(sid), Some(cwd), watch_pid)?;
            }
        }
        "session-end" => {
            if !sid.is_empty() {
                session_end(sid)?;
            }
        }
        "user-prompt-submit" => {
            let prompt: Option<String> = obj
                .and_then(|o| o.get("prompt"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            turn_start(sid.clone(), transcript, json_out).await?;
            // Publish the user's prompt as a kind:1 OP on the Nostr fabric.
            // Fail open: if userNsec is absent or the relay is unreachable, the
            // hook must not block the editor.
            if let Some(prompt_text) = prompt {
                let params = serde_json::json!({
                    "env_session": sid,
                    "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
                    "cwd": cwd.to_string_lossy(),
                    "prompt": prompt_text,
                });
                if let Err(e) = daemon_call_async("user_prompt", params).await {
                    eprintln!("[tenex-edge] user_prompt publish skipped: {e:#}");
                }
            }
        }
        "post-tool-use" => {
            let explicit = if sid.is_empty() { None } else { Some(sid) };
            turn_check(explicit, json_out)?;
        }
        "stop" => {
            if !sid.is_empty() {
                turn_end(sid)?;
            }
        }
        other => {
            // Fail open: unknown hook types are ignored so future harness
            // versions can add hooks without breaking this binary.
            eprintln!("[tenex-edge] unrecognised hook type {other:?} for host {host_name}");
        }
    }
    Ok(())
}

// ── process-tree PID search (for harnesses like Codex that omit their PID) ───

/// Walk the process tree upward looking for an ancestor whose command name
/// contains `needle` (case-insensitive). Returns the first match.
fn find_ancestor_pid(needle: &str) -> Option<i32> {
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
