//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{now_secs, pubkey_short, session_short_code, slugify_host, SessionId};
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
        /// Canonical thread id to reply into (encodes NIP-10 root e-tag on the
        /// published event so the recipient groups the reply into the same thread).
        /// Omit for a new root message (default Phase 6 behavior).
        #[arg(long = "thread", value_name = "THREAD_ID")]
        thread_id: Option<String>,
    },
    /// List threads and messages for a project (Phase 7 Communications plane).
    Threads {
        /// Project slug (defaults to the project resolved from the current directory).
        #[arg(long)]
        project: Option<String>,
        /// Show messages for a specific thread id.
        #[arg(long)]
        thread: Option<String>,
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
    /// Stream all fabric activity as structured events, colorized.
    Tail {
        /// Filter to a single project (default: all projects).
        #[arg(long)]
        project: Option<String>,
        /// Filter to a specific agent slug.
        #[arg(long)]
        agent: Option<String>,
        /// Filter to a specific host.
        #[arg(long)]
        host: Option<String>,
        /// Only show events after this time (unix timestamp or duration like "1h").
        #[arg(long)]
        since: Option<String>,
        /// Number of backfill events from history (default 20; 0 = live only).
        #[arg(long)]
        backfill: Option<u64>,
        /// Show only these categories (comma-separated: msg,sync,turn,stat,join,leave,sess,acl,proj,profile).
        #[arg(long)]
        only: Option<String>,
        /// Hide these categories (comma-separated).
        #[arg(long)]
        exclude: Option<String>,
        /// Also show normally-hidden categories (e.g. profile).
        #[arg(long)]
        include: Option<String>,
        /// Show everything including noise (profile, heartbeats).
        #[arg(long, short = 'v')]
        all: bool,
        /// Compact mode: minimal output.
        #[arg(long, short = 'q')]
        compact: bool,
        /// Use relative timestamps ("12s ago") instead of wall-clock.
        #[arg(long)]
        relative: bool,
        /// Disable Unicode glyphs, use ASCII fallbacks.
        #[arg(long)]
        no_emoji: bool,
        /// Disable ANSI colors.
        #[arg(long)]
        no_color: bool,
        /// Output raw NDJSON instead of human-readable lines.
        #[arg(long)]
        json: bool,
        /// Stop after history dump (do not follow live events).
        #[arg(long)]
        no_follow: bool,
        /// Full-screen live TUI dashboard (follow-up feature, not yet implemented).
        #[arg(long)]
        live: bool,
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
    /// Publish a long-form proposal (kind:30023) from this agent's session.
    Propose {
        /// Proposal title.
        #[arg(long)]
        title: String,
        /// Proposal body (Markdown). Use "-" or omit to read from stdin.
        #[arg(long = "message", value_name = "BODY")]
        message: Option<String>,
        /// Optional canonical thread id to attach this proposal to.
        #[arg(long = "thread", value_name = "THREAD_ID")]
        thread_id: Option<String>,
        /// Stable addressable identifier (the kind:30023 `d` tag). Reuse the same
        /// value to publish a REVISION that supersedes a prior proposal at the
        /// same address. Omit to mint a fresh id (a new proposal).
        #[arg(long = "d", value_name = "IDENTIFIER")]
        d: Option<String>,
        /// My session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
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
    /// Add a pubkey to a project's NIP-29 group (kind:9000 put-user).
    /// Accepts hex pubkey, npub (bech32), or a NIP-05 address (user@domain.com).
    Add {
        /// Project slug.
        project: String,
        /// Hex pubkey, npub, or NIP-05 address.
        #[arg(value_name = "PUBKEY")]
        pubkey: String,
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
            thread_id,
        } => {
            let recipient = recipient_flag
                .or(recipient)
                .context("missing recipient; use `tenex-edge send-message --recipient <target> --message \"...\"`")?;
            let message = resolve_send_message_body(message_flag.or(message))?;
            send_message(recipient, message, session, thread_id).await
        }
        Cmd::Propose { title, message, thread_id, d, session } => {
            let body = resolve_send_message_body(message)?;
            propose(title, body, thread_id, d, session).await
        }
        Cmd::Threads { project, thread } => threads(project, thread).await,
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
        Cmd::Tail {
            project, agent, host, since, backfill,
            only, exclude, include, all, compact,
            relative, no_emoji, no_color, json, no_follow, live,
        } => tail(TailOpts {
            project, agent, host, since, backfill,
            only, exclude, include, all, compact,
            relative, no_emoji, no_color, json, no_follow, live,
        }).await,
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

async fn send_message(
    recipient: String,
    message: String,
    session: Option<String>,
    thread_id: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "recipient": recipient,
        "message": message,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "thread_id": thread_id,
    });
    let v = daemon_call_async("send_message", params).await?;
    let to_pubkey = v["to_pubkey"].as_str().unwrap_or_default().to_string();
    let target_session = v["target_session"].as_str().map(str::to_string);
    match target_session {
        Some(s) => println!("mentioned {} (session {})", pubkey_short(&to_pubkey), pubkey_short(&s)),
        None => println!("mentioned {}", pubkey_short(&to_pubkey)),
    }
    Ok(())
}

// ── propose ───────────────────────────────────────────────────────────────────

async fn propose(
    title: String,
    body: String,
    thread_id: Option<String>,
    d: Option<String>,
    session: Option<String>,
) -> Result<()> {
    let params = serde_json::json!({
        "title": title,
        "body": body,
        "session": session,
        "env_session": std::env::var("TENEX_EDGE_SESSION").ok(),
        "agent": std::env::var("TENEX_EDGE_AGENT").ok(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "thread_id": thread_id,
        "d": d,
    });
    let v = daemon_call_async("propose", params).await?;
    let title_echo = v["title"].as_str().unwrap_or(&title);
    let d_tag = v["d_tag"].as_str().unwrap_or("?");
    println!("published proposal {} ({})", title_echo, d_tag);
    Ok(())
}

// ── threads ───────────────────────────────────────────────────────────────────

/// `threads`: list threads for a project, or messages for a specific thread.
///
/// Routes to the daemon via `list_threads`, `messages`, or `thread_meta` RPCs
/// and prints a human-readable summary.
async fn threads(project: Option<String>, thread: Option<String>) -> Result<()> {
    if let Some(tid) = thread {
        // Show messages for a specific thread.
        let v = daemon_call_async("messages", serde_json::json!({ "thread_id": tid })).await?;
        let meta_v = daemon_call_async("thread_meta", serde_json::json!({ "thread_id": tid })).await?;

        if let Some(subject) = meta_v.get("subject").and_then(|v| v.as_str()) {
            println!("Thread: {}", subject);
        } else {
            println!("Thread: {}", pubkey_short(&tid));
        }
        if let Some(msgs) = v.as_array() {
            for msg in msgs {
                let dir = msg["direction"].as_str().unwrap_or("?");
                let author = msg["author_pubkey"].as_str().unwrap_or("?");
                let body = msg["body"].as_str().unwrap_or("");
                let ts = msg["created_at"].as_u64().unwrap_or(0);
                let arrow = if dir == "outbound" { "->" } else { "<-" };
                println!("[{}] {} {} {}: {}", ts, pubkey_short(author), arrow, dir, body);
            }
        }
        return Ok(());
    }

    // List threads for a project.
    let proj = project.unwrap_or_else(|| {
        crate::project::resolve(&std::env::current_dir().unwrap_or_default())
    });
    let v = daemon_call_async("list_threads", serde_json::json!({ "project": proj })).await?;
    if let Some(threads) = v.as_array() {
        if threads.is_empty() {
            println!("No threads in project {:?}", proj);
            return Ok(());
        }
        println!("Threads in {}:", proj);
        for t in threads {
            let tid = t["thread_id"].as_str().unwrap_or("?");
            let count = t["message_count"].as_u64().unwrap_or(0);
            let last = t["last_message_at"].as_u64();
            let subject = t["subject"].as_str();
            let label = subject.unwrap_or("no subject");
            match last {
                // Print the FULL thread id — it is the argument the user passes
                // back to `threads --thread <id>`; a pubkey_short() would be unusable.
                Some(ts) => println!("  {} ({} msg, last at {}) - {}", tid, count, ts, label),
                None => println!("  {} (no messages) - {}", tid, label),
            }
        }
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
        if event::poll(wait)? && should_quit_live(event::read()?) {
            break;
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

    // Route through Phase 2 read-model methods so Phase 8 can swap the source
    // without touching this function.
    let mine = store.list_agents_read_model(None, since)?;
    let my_ids: std::collections::HashSet<String> =
        mine.iter().map(|s| s.session_id.clone()).collect();
    let all_peers: Vec<_> = store
        .list_presence_read_model(None, since)?
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
            // Route through the read-model method so Phase 8 can swap the source.
            let about = store.project_meta_read_model(&project).ok().flatten();
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
                pubkey_short(&p.session_id),
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
        assert!(once.contains(&format!(
            "coder [session {}] - idle",
            session_short_code("local-session")
        )));
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

        assert!(once.contains(&format!("a [session {}] - idle", session_short_code("s1"))));
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
        assert!(once.contains(&format!(
            "reviewer@other [session {}] - idle",
            session_short_code("remote-session")
        )));
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
                pubkey_short(v["pubkey"].as_str().unwrap_or(""))
            );
        }
        Some(AclAction::Block { target }) => {
            let v = daemon_call_async("acl", serde_json::json!({"action": "block", "target": target})).await?;
            println!(
                "blocked {} ({})",
                v["slug"].as_str().unwrap_or(""),
                pubkey_short(v["pubkey"].as_str().unwrap_or(""))
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
                        pubkey_short(p["pubkey"].as_str().unwrap_or("")),
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
                    pubkey_short(p["pubkey"].as_str().unwrap_or(""))
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

        // Warn if this agent couldn't be added to the NIP-29 group (e.g. the
        // daemon on this machine is not the relay admin). The session-start hook
        // tried and failed silently; surface it here so the agent can tell the
        // user what to fix.
        let not_member = {
            let s = store.lock().expect("store mutex poisoned");
            !s.is_group_member(&rec.project, &rec.agent_pubkey)
                .unwrap_or(true)
        };
        if not_member {
            blocks.push(format!(
                "[tenex-edge] WARNING: this agent ({slug}, pubkey {pubkey}) \
                 is not a member of the NIP-29 group for project \"{project}\". \
                 Messages published by this session may be rejected by the relay. \
                 Tell the user to run the following command from a machine that \
                 has relay admin access (e.g. where this project was first set up):\n\
                 \n  tenex-edge project add {project} {pubkey}",
                slug = rec.agent_slug,
                pubkey = rec.agent_pubkey,
                project = rec.project,
            ));
        }
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
            .map(|p| format!("{} ({})", p.slug, pubkey_short(&p.pubkey)))
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
        // Route through the read-model method (peek semantics preserved).
        let rows = s.undelivered_messages_for_session(session_id).unwrap_or_default();
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
            pubkey_short(&row.session_id),
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
        ProjectAction::Add { project, pubkey } => {
            let v = daemon_call_async(
                "project_add",
                serde_json::json!({ "project": project, "pubkey": pubkey }),
            )
            .await?;
            let resolved = v["pubkey"].as_str().unwrap_or(&pubkey);
            println!(
                "added {} to {}",
                pubkey_short(resolved).cyan(),
                project.bold()
            );
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

/// Options for the `tail` command.
struct TailOpts {
    project: Option<String>,
    agent: Option<String>,
    host: Option<String>,
    since: Option<String>,
    backfill: Option<u64>,
    only: Option<String>,
    exclude: Option<String>,
    include: Option<String>,
    all: bool,
    compact: bool,
    relative: bool,
    no_emoji: bool,
    no_color: bool,
    json: bool,
    no_follow: bool,
    live: bool,
}

async fn tail(opts: TailOpts) -> Result<()> {
    if opts.live {
        eprintln!(
            "tenex-edge tail --live: the full-screen TUI dashboard is not yet implemented. \
             Use bare `tenex-edge tail` for the live scrolling feed."
        );
        return Ok(());
    }

    // Resolve color + emoji settings: explicit flags override env/TTY.
    let use_color = !opts.no_color
        && std::env::var("NO_COLOR").is_err()
        && std::io::stdout().is_terminal();
    let use_emoji = !opts.no_emoji;

    // Parse --since into a unix timestamp.
    let since_ts: u64 = opts.since.as_deref().map(parse_since).unwrap_or(0);

    let scope_label = opts.project.as_deref().unwrap_or("*");
    if !opts.json {
        eprintln!(
            "{} tailing project {} … (Ctrl-C to stop)",
            if use_color { "tenex-edge".bold().to_string() } else { "tenex-edge".to_string() },
            if use_color { scope_label.cyan().to_string() } else { scope_label.to_string() },
        );
    }

    // Build the category filter set.
    let cats_only: Option<std::collections::HashSet<String>> = opts.only.as_deref().map(|s| {
        s.split(',').map(|c| c.trim().to_lowercase()).collect()
    });
    let cats_exclude: std::collections::HashSet<String> = opts.exclude.as_deref().map(|s| {
        s.split(',').map(|c| c.trim().to_lowercase()).collect()
    }).unwrap_or_default();
    let cats_include: std::collections::HashSet<String> = opts.include.as_deref().map(|s| {
        s.split(',').map(|c| c.trim().to_lowercase()).collect()
    }).unwrap_or_default();

    // Minimum tier: default hides tier 0 (profile); --all includes all; --v same.
    let min_tier: u8 = if opts.all { 0 } else { 1 };

    let params = serde_json::json!({
        "project": opts.project,
        "backfill": opts.backfill.unwrap_or(20),
        "since": since_ts,
    });

    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;

    let agent_filter = opts.agent.clone();
    let host_filter = opts.host.clone();
    let is_json = opts.json;
    let no_follow = opts.no_follow;
    let compact = opts.compact;
    let relative = opts.relative;

    let stream = client.stream("tail", params, move |item| {
        // Deserialize TailEvent.
        let ev: crate::daemon::tail_event::TailEvent = match serde_json::from_value(item.clone()) {
            Ok(e) => e,
            Err(_) => {
                // Fallback: if we get an old {line} format, print it.
                if let Some(line) = item.get("line").and_then(|l| l.as_str()) {
                    println!("{line}");
                }
                return;
            }
        };

        // Apply agent/host filters.
        if let Some(ref ag) = agent_filter {
            let ev_agent = match &ev {
                crate::daemon::tail_event::TailEvent::Msg { from, .. } => from.as_str(),
                crate::daemon::tail_event::TailEvent::Turn { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Status { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Join { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Sess { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Acl { agent, .. } => agent.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { agent, .. } => agent.as_str(),
                _ => "",
            };
            if !ev_agent.is_empty() && ev_agent != ag.as_str() {
                return;
            }
        }
        if let Some(ref h) = host_filter {
            let ev_host = match &ev {
                crate::daemon::tail_event::TailEvent::Join { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Leave { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Acl { host, .. } => host.as_str(),
                crate::daemon::tail_event::TailEvent::Profile { host, .. } => host.as_str(),
                _ => "",
            };
            if !ev_host.is_empty() && ev_host != h.as_str() {
                return;
            }
        }

        // Tier filter.
        if ev.tier() < min_tier && !cats_include.contains(ev.category()) {
            return;
        }

        // Category filters.
        let cat = ev.category();
        if let Some(ref only) = cats_only {
            if !only.contains(cat) {
                return;
            }
        }
        if cats_exclude.contains(cat) && !cats_include.contains(cat) {
            return;
        }

        // Render.
        if is_json {
            if let Ok(s) = serde_json::to_string(&ev) {
                println!("{s}");
            }
        } else {
            let line = render_tail_event(&ev, use_color, use_emoji, relative, compact);
            println!("{line}");
        }
    });

    if no_follow {
        // For no-follow: run with a short timeout to get just the backfill.
        // The daemon will keep streaming; we disconnect after receiving the
        // initial batch. Since we can't easily detect "backfill done", we
        // use a small sleep approach: connect, get backfill, disconnect.
        tokio::select! {
            r = stream => r,
            _ = tokio::time::sleep(Duration::from_millis(500)) => Ok(()),
        }
    } else {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Ok(()),
            r = stream => r,
        }
    }
}

/// Parse a --since value into a unix timestamp.
/// Accepts: unix seconds ("1700000000"), or durations ("1h", "30m", "2d").
fn parse_since(s: &str) -> u64 {
    let now = now_secs();
    if let Ok(ts) = s.parse::<u64>() {
        return ts;
    }
    // Simple duration parsing: Nh, Nm, Nd, Ns.
    let s = s.trim();
    let (num_str, unit) = s.split_at(s.len().saturating_sub(1));
    if let Ok(n) = num_str.trim().parse::<u64>() {
        let secs = match unit {
            "h" | "H" => n * 3600,
            "m" | "M" => n * 60,
            "d" | "D" => n * 86400,
            "s" | "S" | _ => n,
        };
        return now.saturating_sub(secs);
    }
    0
}

/// Render a `TailEvent` to a human-readable string.
///
/// `use_color` and `use_emoji` are passed explicitly so this fn is testable
/// without side-effects from TTY detection or NO_COLOR.
pub fn render_tail_event(
    ev: &crate::daemon::tail_event::TailEvent,
    use_color: bool,
    use_emoji: bool,
    relative: bool,
    compact: bool,
) -> String {
    use crate::daemon::tail_event::TailEvent;
    use crate::util::session_short_code;

    let ts = ev.ts();
    let ts_str = if relative {
        let age = now_secs().saturating_sub(ts);
        if age < 60 {
            format!("{age}s ago")
        } else if age < 3600 {
            format!("{}m ago", age / 60)
        } else {
            format!("{}h ago", age / 3600)
        }
    } else {
        // Wall-clock HH:MM:SS.
        let h = (ts % 86400) / 3600;
        let m = (ts % 3600) / 60;
        let s = ts % 60;
        format!("{h:02}:{m:02}:{s:02}")
    };

    // Helper: colorize if color enabled.
    macro_rules! col {
        ($text:expr, $color:ident) => {
            if use_color {
                $text.$color().to_string()
            } else {
                $text.to_string()
            }
        };
    }

    // Session short code helper.
    let sess_code = |sid: &str| session_short_code(sid);

    match ev {
        TailEvent::Msg { project, from, from_session, to, to_session, thread, body, .. } => {
            let cat = col!("msg  ", yellow);
            let arrow = if use_emoji { "→" } else { "->" };
            let sess = from_session.as_deref().map(|s| format!("[{}]", sess_code(s))).unwrap_or_default();
            let to_sess = to_session.as_deref().map(|s| format!("[{}]", sess_code(s))).unwrap_or_default();
            let thread_tag = thread.as_deref().map(|t| format!(" #{}", &t[..t.len().min(8)])).unwrap_or_default();
            let snippet = if compact {
                String::new()
            } else {
                let body_clean: String = body.chars().take(72).collect();
                let body_clean = body_clean.replace('\n', " ");
                let ellipsis = if body.len() > 72 { "…" } else { "" };
                format!(" \"{}{}\"", body_clean, ellipsis)
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {arrow} {}{to_sess}{thread_tag}{snippet}",
                col!(from, cyan),
                col!(to, cyan),
            )
        }

        TailEvent::Sync { from, to, thread, state, .. } => {
            let (cat, color_fn): (&str, fn(&str) -> String) = match state.as_str() {
                "failed" => ("sync ", |s| {
                    if true { s.red().to_string() } else { s.to_string() }
                }),
                _ => ("sync ", |s| s.cyan().to_string()),
            };
            let cat_str = if use_color {
                match state.as_str() {
                    "failed" => col!(cat, red),
                    _ => col!(cat, cyan),
                }
            } else {
                cat.to_string()
            };
            let _ = color_fn; // suppress unused warning
            let thread_tag = thread.as_deref().map(|t| format!(" #{}", &t[..t.len().min(8)])).unwrap_or_default();
            let glyph = if use_emoji {
                match state.as_str() { "delivered" => "✓", "failed" => "✗", _ => "~" }
            } else {
                match state.as_str() { "delivered" => "[ok]", "failed" => "[x]", _ => "~" }
            };
            format!("{ts_str}  {cat_str}  {from} → {to}{thread_tag}  {glyph} {state}")
        }

        TailEvent::Turn { project, agent, session, state, elapsed_s, .. } => {
            let cat = col!("turn ", green);
            let sess = format!("[{}]", sess_code(session));
            let (glyph, detail) = if state == "working" {
                let g = if use_emoji { "▶" } else { ">" };
                (g, " started working".to_string())
            } else {
                let g = if use_emoji { "⏸" } else { "||" };
                let dur = elapsed_s.map(|e| format!(" ({})", fmt_duration(e))).unwrap_or_default();
                (g, format!(" idle{dur}"))
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  {glyph}{detail}",
                col!(agent, cyan),
            )
        }

        TailEvent::Status { project, agent, text, .. } => {
            let cat = col!("stat ", magenta);
            let label = if text.is_empty() { "idle" } else { text.as_str() };
            format!("{ts_str}  {cat}  {}@{project}  {label}", col!(agent, cyan))
        }

        TailEvent::Join { project, agent, host, session, rel_cwd, .. } => {
            let cat = col!("join ", green);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" ({})", rel_cwd)
            };
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  online ({project}{cwd_info})",
                col!(agent, cyan),
            )
        }

        TailEvent::Leave { project, agent, host, session, online_s, .. } => {
            let cat = col!("leave", dimmed);
            let sess = format!("[{}]", sess_code(session));
            let dur = fmt_duration(*online_s);
            format!(
                "{ts_str}  {cat}  {}@{host}{sess}  offline (was online {dur}, {project})",
                col!(agent, cyan),
            )
        }

        TailEvent::Sess { project, agent, session, state, rel_cwd, .. } => {
            let cat = col!("sess ", blue);
            let sess = format!("[{}]", sess_code(session));
            let cwd_info = if rel_cwd.is_empty() || rel_cwd == "." {
                String::new()
            } else {
                format!(" (rel_cwd: {rel_cwd})")
            };
            format!(
                "{ts_str}  {cat}  {}@{project}{sess}  session {state}{cwd_info}",
                col!(agent, cyan),
            )
        }

        TailEvent::Acl { action, agent, host, pubkey, role, .. } => {
            let cat = match action.as_str() {
                "pending" => {
                    if use_color { "acl  ".bold().yellow().to_string() } else { "acl  ".to_string() }
                }
                "admitted" => col!("acl  ", green),
                _ => col!("acl  ", red),
            };
            let glyph = if use_emoji {
                match action.as_str() { "admitted" => "✓", "pending" => "⚠", _ => "✗" }
            } else {
                match action.as_str() { "admitted" => "[ok]", "pending" => "[!]", _ => "[x]" }
            };
            let role_str = role.as_deref().map(|r| format!(" ({r})")).unwrap_or_default();
            let pk_short = &pubkey[..pubkey.len().min(8)];
            format!(
                "{ts_str}  {cat}  {}@{host}  {glyph} {action}{role_str} ({})",
                col!(agent, cyan),
                pk_short,
            )
        }

        TailEvent::Proj { project, about, .. } => {
            let cat = col!("proj ", dimmed);
            let snippet: String = about.chars().take(60).collect();
            format!("{ts_str}  {cat}  {project}  {snippet}")
        }

        TailEvent::Profile { agent, host, pubkey, .. } => {
            let cat = col!("id   ", dimmed);
            let pk_short = &pubkey[..pubkey.len().min(8)];
            format!("{ts_str}  {cat}  {}@{host}  ({pk_short})", col!(agent, cyan))
        }
    }
}

fn fmt_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
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
            format!("{}", p.session_id).yellow(),
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
            pubkey_short(&m.to_pubkey),
            m.target_session
                .as_ref()
                .map(|s| format!(" ({s})"))
                .unwrap_or_default(),
            m.body
        ),
        DomainEvent::TurnReply(r) => {
            format!("{} {}: {}", "turn".blue(), r.agent.slug.cyan(), r.body)
        }
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

// ── freeze tests — turn-start / turn-check context assembly ─────────────────

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::state::{InboxRow, SessionRecord, Store};
    use std::sync::Mutex;

    /// Build a minimal alive SessionRecord (not first-turn when prev != 0, no peers
    /// seeded, so the only context block the function can emit is inbox mentions).
    fn test_session(id: &str) -> SessionRecord {
        SessionRecord {
            session_id: id.to_string(),
            agent_slug: "coder".to_string(),
            agent_pubkey: "pk-coder".to_string(),
            project: "proj".to_string(),
            host: "laptop".to_string(),
            child_pid: None,
            watch_pid: None,
            created_at: 1,
            alive: true,
            rel_cwd: String::new(),
        }
    }

    fn inbox_row(session_id: &str, eid: &str) -> InboxRow {
        InboxRow {
            mention_event_id: eid.to_string(),
            target_session: session_id.to_string(),
            from_pubkey: "pk-sender".to_string(),
            from_slug: "sender".to_string(),
            project: "proj".to_string(),
            body: "hello from sender".to_string(),
            created_at: 100,
            from_session: String::new(),
        }
    }

    /// FREEZE C1: assemble_turn_start_context drains inbox rows and renders them.
    ///
    /// On a non-first turn (prev_turn_started_at != 0) with no peer sessions seeded,
    /// the ONLY possible context block is inbox mentions. With one row present: the
    /// function returns Some(text) containing the mention line. On a SECOND call
    /// (the row is now delivered=1), it returns None — the drain was real.
    #[test]
    fn freeze_turn_start_context_drains_inbox_and_renders_mention_line() {
        let store = Store::open_memory().unwrap();
        let rec = test_session("sess-freeze-1");

        // Seed one inbox row for this session.
        store.enqueue_mention(&inbox_row("sess-freeze-1", "evt-c1")).unwrap();

        let m = Mutex::new(store);

        // Non-first turn (prev != 0) → no intro block; no peers → no fabric block.
        // Only the inbox mention block should be present.
        let ctx = assemble_turn_start_context(&m, &rec, /* prev_turn_started_at */ 1);
        let text = ctx.expect("FREEZE: turn_start must return Some when inbox has rows");

        assert!(
            text.contains("Messages from other agents (tenex-edge):"),
            "FREEZE: mention section header must be present; got: {text:?}"
        );
        assert!(
            text.contains("[mention from sender@proj"),
            "FREEZE: mention line must contain [mention from sender@proj; got: {text:?}"
        );
        assert!(
            text.contains("hello from sender"),
            "FREEZE: mention body must be in context; got: {text:?}"
        );

        // SECOND call: the drain marked the row delivered — no more context to emit.
        let ctx2 = assemble_turn_start_context(&m, &rec, /* prev_turn_started_at */ 1);
        assert!(
            ctx2.is_none(),
            "FREEZE: second turn_start call must return None (row already drained)"
        );
    }

    /// FREEZE C2: assemble_turn_start_context returns None when inbox is empty
    /// (non-first turn, no peers).
    #[test]
    fn freeze_turn_start_context_returns_none_when_inbox_empty_non_first_turn() {
        let store = Store::open_memory().unwrap();
        let rec = test_session("sess-freeze-2");
        // No inbox rows. Non-first turn (prev != 0). No peer sessions.
        let m = Mutex::new(store);

        let ctx = assemble_turn_start_context(&m, &rec, /* prev_turn_started_at */ 42);
        assert!(
            ctx.is_none(),
            "FREEZE: turn_start with empty inbox, non-first turn, no peers must return None"
        );
    }

    /// FREEZE C3: assemble_turn_check_context PEEKs — rows survive and are still
    /// drainable by turn_start afterward.
    ///
    /// This is the discriminating property: peek does NOT set delivered=1, so a
    /// following drain_inbox still finds the row.
    #[test]
    fn freeze_turn_check_context_peeks_not_drains() {
        let store = Store::open_memory().unwrap();
        store.enqueue_mention(&inbox_row("sess-freeze-3", "evt-c3")).unwrap();
        let m = Mutex::new(store);

        // turn_check peeks: returns Some with the mention line.
        let ctx = assemble_turn_check_context(&m, "sess-freeze-3");
        let text = ctx.expect("FREEZE: turn_check must return Some when inbox has undelivered rows");
        assert!(
            text.contains("[tenex-edge] Message(s) arrived while you were working:"),
            "FREEZE: turn_check header must be present; got: {text:?}"
        );
        assert!(
            text.contains("[mention from sender@proj"),
            "FREEZE: turn_check must render the mention line; got: {text:?}"
        );

        // The row is still in the store (peek, not drain): drain_inbox now consumes it.
        let g = m.lock().unwrap();
        let drained = g.drain_inbox("sess-freeze-3").unwrap();
        assert_eq!(
            drained.len(), 1,
            "FREEZE: row must survive turn_check peek and still be drainable"
        );
    }

    /// FREEZE C4: assemble_turn_check_context returns None when inbox is empty.
    #[test]
    fn freeze_turn_check_context_returns_none_when_inbox_empty() {
        let store = Store::open_memory().unwrap();
        let m = Mutex::new(store);
        let ctx = assemble_turn_check_context(&m, "sess-no-rows");
        assert!(
            ctx.is_none(),
            "FREEZE: turn_check with empty inbox must return None"
        );
    }

    /// FREEZE C5: reply-to handle falls back to slug@project when from_session
    /// is empty (the sender's session id is unknown — old peers / untargeted).
    #[test]
    fn freeze_mention_reply_handle_falls_back_to_slug_at_project() {
        let store = Store::open_memory().unwrap();
        let row = InboxRow {
            mention_event_id: "evt-handle".to_string(),
            target_session: "sess-x".to_string(),
            from_pubkey: "pk-s".to_string(),
            from_slug: "reviewer".to_string(),
            project: "myproj".to_string(),
            body: "yo".to_string(),
            created_at: 1,
            from_session: String::new(), // unknown session id
        };
        let handle = mention_reply_handle(&store, &row);
        assert_eq!(
            handle, "reviewer@myproj",
            "FREEZE: empty from_session must fall back to slug@project"
        );
    }

    /// FREEZE C6: reply-to handle uses from_session when it resolves in the store
    /// (peer session prefix lookup succeeds → exact session id is returned).
    #[test]
    fn freeze_mention_reply_handle_uses_session_when_resolvable() {
        let store = Store::open_memory().unwrap();
        // Register the sender's session in peer_sessions so prefix lookup resolves.
        store
            .upsert_peer_session("sender-session-id", "pk-s", "reviewer", "myproj", "host", "", 1000)
            .unwrap();

        let row = InboxRow {
            mention_event_id: "evt-handle-2".to_string(),
            target_session: "sess-x".to_string(),
            from_pubkey: "pk-s".to_string(),
            from_slug: "reviewer".to_string(),
            project: "myproj".to_string(),
            body: "yo".to_string(),
            created_at: 1,
            from_session: "sender-session-id".to_string(),
        };
        let handle = mention_reply_handle(&store, &row);
        assert_eq!(
            handle, "sender-session-id",
            "FREEZE: resolvable from_session must be used as reply-to handle"
        );
    }
}

// ── tail render tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tail_render_tests {
    use super::*;
    use crate::daemon::tail_event::TailEvent;

    const TS: u64 = 1_700_000_000; // 2023-11-14 22:13:20 UTC  → 22:13:20 wall-clock

    fn ts_str() -> String {
        let h = (TS % 86400) / 3600;
        let m = (TS % 3600) / 60;
        let s = TS % 60;
        format!("{h:02}:{m:02}:{s:02}")
    }

    // ── Msg ─────────────────────────────────────────────────────────────────

    #[test]
    fn render_msg_no_color_no_emoji() {
        let ev = TailEvent::Msg {
            ts: TS,
            project: "proj".into(),
            from: "claude".into(),
            from_session: Some("te-abc-111".into()),
            to: "codex".into(),
            to_session: None,
            thread: Some("b8e2".into()),
            body: "can you review the codec?".into(),
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.starts_with(&ts_str()), "should start with timestamp");
        assert!(line.contains("msg"), "should contain category");
        assert!(line.contains("claude@proj"), "should contain agent@project");
        assert!(line.contains("->"), "ASCII arrow when no_emoji");
        assert!(line.contains("codex"), "should contain recipient");
        assert!(line.contains("#b8e2"), "should contain thread");
        assert!(line.contains("review the codec"), "should contain body");
    }

    #[test]
    fn render_msg_with_emoji() {
        let ev = TailEvent::Msg {
            ts: TS,
            project: "proj".into(),
            from: "claude".into(),
            from_session: None,
            to: "codex".into(),
            to_session: None,
            thread: None,
            body: "hello".into(),
        };
        let line = render_tail_event(&ev, false, true, false, false);
        assert!(line.contains("→"), "Unicode arrow when emoji enabled");
    }

    // ── Turn ─────────────────────────────────────────────────────────────────

    #[test]
    fn render_turn_working_no_color() {
        let ev = TailEvent::Turn {
            ts: TS,
            project: "proj".into(),
            agent: "claude".into(),
            session: "te-session-1".into(),
            state: "working".into(),
            elapsed_s: None,
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("turn"), "category");
        assert!(line.contains("claude@proj"), "agent@project");
        assert!(line.contains("started working"), "state label");
        assert!(line.contains(">"), "ASCII glyph when no emoji");
    }

    #[test]
    fn render_turn_idle_with_elapsed() {
        let ev = TailEvent::Turn {
            ts: TS,
            project: "proj".into(),
            agent: "claude".into(),
            session: "te-session-1".into(),
            state: "idle".into(),
            elapsed_s: Some(91),
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("idle"), "should contain idle label");
        assert!(line.contains("1m31s"), "should contain formatted duration");
    }

    // ── Join / Leave ─────────────────────────────────────────────────────────

    #[test]
    fn render_join_no_color() {
        let ev = TailEvent::Join {
            ts: TS,
            project: "tenex-edge".into(),
            agent: "codex".into(),
            host: "tower".into(),
            session: "te-peer-abc".into(),
            rel_cwd: ".".into(),
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("join"), "category");
        assert!(line.contains("codex@tower"), "agent@host");
        assert!(line.contains("online"), "verb");
        assert!(line.contains("tenex-edge"), "project");
    }

    #[test]
    fn render_leave_formats_duration() {
        let ev = TailEvent::Leave {
            ts: TS,
            project: "proj".into(),
            agent: "opencode".into(),
            host: "tower".into(),
            session: "te-peer-def".into(),
            online_s: 1020,
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("leave"), "category");
        assert!(line.contains("offline"), "verb");
        assert!(line.contains("17m0s"), "duration 1020s = 17m0s");
    }

    // ── Acl ──────────────────────────────────────────────────────────────────

    #[test]
    fn render_acl_pending_no_color() {
        let ev = TailEvent::Acl {
            ts: TS,
            action: "pending".into(),
            agent: "stranger".into(),
            host: "unknown".into(),
            pubkey: "abcdef1234567890".into(),
            role: None,
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("acl"), "category");
        assert!(line.contains("[!]"), "ASCII pending glyph");
        assert!(line.contains("stranger@unknown"), "agent@host");
        assert!(line.contains("abcdef12"), "pubkey prefix");
    }

    // ── Sess ─────────────────────────────────────────────────────────────────

    #[test]
    fn render_sess_start_no_color() {
        let ev = TailEvent::Sess {
            ts: TS,
            project: "proj".into(),
            agent: "claude".into(),
            session: "te-abc-999".into(),
            state: "start".into(),
            rel_cwd: ".".into(),
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.contains("sess"), "category");
        assert!(line.contains("session start"), "state label");
    }

    // ── parse_since ──────────────────────────────────────────────────────────

    #[test]
    fn parse_since_unix_passthrough() {
        assert_eq!(parse_since("1700000000"), 1_700_000_000);
    }

    #[test]
    fn parse_since_duration_h() {
        let now = now_secs();
        let result = parse_since("1h");
        let expected = now.saturating_sub(3600);
        // Allow ±2s for timing.
        assert!((result as i64 - expected as i64).abs() <= 2, "1h parse");
    }

    #[test]
    fn parse_since_zero_for_garbage() {
        assert_eq!(parse_since("not-a-time"), 0);
    }
}
