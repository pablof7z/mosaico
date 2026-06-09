//! The host-neutral CLI surface (M1 §6).

use crate::codec::{Codec, Kind1Codec, SubScope};
use crate::config::{self, Config};
use crate::domain::{AgentRef, DomainEvent, Mention};
use crate::identity;
use crate::project;
use crate::runtime::{self, EngineParams};
use crate::state::Store;
use crate::transport::Transport;
use crate::util::{now_secs, short_id, slugify_host};
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use nostr_sdk::prelude::RelayPoolNotification;
use owo_colors::OwoColorize;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// Start a session: publish identity, begin presence/awareness in the background.
    SessionStart {
        #[arg(long)]
        agent: String,
        /// Adopt the host's session id (e.g. Claude Code's). Generated if absent.
        #[arg(long)]
        session_id: Option<String>,
        /// Working directory to resolve the project from (default: cwd).
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Host PID to watch; the background process stops if it dies.
        #[arg(long)]
        watch_pid: Option<i32>,
    },
    /// End a session cleanly (stops the background process, goes idle).
    SessionEnd {
        #[arg(long)]
        session: String,
    },
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
    /// Print + drain pending mentions for a session (used by the injection hook).
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
    /// Mark a session as working on a turn (used by the turn-start hook, e.g.
    /// Claude Code's UserPromptSubmit). Outputs fabric context for the agent:
    /// inbox messages, and presence/status changes since the last turn.
    TurnStart {
        #[arg(long)]
        session: String,
        /// Path to the host conversation transcript (JSONL) for this session.
        #[arg(long)]
        transcript: Option<String>,
        /// Emit JSON {"systemMessage": "..."} instead of plain text (for Codex).
        #[arg(long)]
        json: bool,
    },
    /// Check for new inbox messages mid-turn (used by PostToolUse hook).
    /// Read-only: does not drain the inbox; turn-start at the next prompt drains.
    TurnCheck {
        /// Session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
        /// Emit JSON {"systemMessage": "..."} instead of plain text (for Codex).
        #[arg(long)]
        json: bool,
    },
    /// Mark a session idle (used by the turn-end hook, e.g. Claude Code's Stop).
    /// The engine clears the agent's status on its next poll.
    TurnEnd {
        #[arg(long)]
        session: String,
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
    /// Internal: the detached per-session engine. Not for direct use.
    #[command(name = "__run-session", hide = true)]
    RunSession {
        #[arg(long)]
        agent: String,
        #[arg(long)]
        session_id: String,
        #[arg(long)]
        project: String,
        #[arg(long)]
        watch_pid: Option<i32>,
    },
    /// Internal: the per-machine daemon. Spawned automatically; not for direct use.
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

pub async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        Cmd::SessionStart {
            agent,
            session_id,
            cwd,
            watch_pid,
        } => session_start(agent, session_id, cwd, watch_pid),
        Cmd::SessionEnd { session } => session_end(session),
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
        Cmd::Doctor => doctor().await,
        Cmd::TurnStart {
            session,
            transcript,
            json,
        } => turn_start(session, transcript, json).await,
        Cmd::TurnCheck { session, json } => turn_check(session, json),
        Cmd::TurnEnd { session } => turn_end(session),
        Cmd::Hook { host, hook_type } => hook_run(host, hook_type).await,
        Cmd::RunSession {
            agent,
            session_id,
            project,
            watch_pid,
        } => run_session(agent, session_id, project, watch_pid).await,
        Cmd::Daemon => crate::daemon::server::run().await,
    }
}

/// A peer is "live" only while heartbeats keep it fresh (3× the default 30s tick).
const PEER_FRESH_SECS: u64 = 90;

fn store_path() -> PathBuf {
    config::edge_home().join("state.db")
}

fn open_store() -> Result<Store> {
    Store::open(&store_path())
}

/// Resolve the caller's session: explicit id if given, else the most-recent
/// alive session for the project of the current directory. Lets agents that
/// don't know their session id just run `tenex-edge inbox` / `send-message`.
fn resolve_session(store: &Store, explicit: Option<String>) -> Result<crate::state::SessionRecord> {
    if let Some(id) = explicit {
        return store
            .get_session(&id)?
            .with_context(|| format!("unknown session {id}"));
    }
    // Host adapters can export this so an agent resolves ITS OWN session even
    // when several agents share a project.
    if let Ok(id) = std::env::var("TENEX_EDGE_SESSION") {
        if !id.is_empty() {
            if let Some(rec) = store.get_session(&id)? {
                return Ok(rec);
            }
        }
    }
    let project = project::resolve(&std::env::current_dir()?);
    store
        .latest_alive_session_for_project(&project)?
        .with_context(|| format!("no active tenex-edge session for project {project:?} (run session-start, or pass --session)"))
}

fn gen_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("te-{:x}-{}", nanos, std::process::id())
}

// ── session-start ────────────────────────────────────────────────────────────

fn session_start(
    agent: String,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    watch_pid: Option<i32>,
) -> Result<()> {
    println!("{}", session_start_inner(agent, session_id, cwd, watch_pid)?);
    Ok(())
}

/// Core session-start logic. Returns the resolved session id.
/// Callers decide what to do with it (print for CLI, discard for hooks).
fn session_start_inner(
    agent: String,
    session_id: Option<String>,
    cwd: Option<PathBuf>,
    watch_pid: Option<i32>,
) -> Result<String> {
    let cfg = Config::load().context("loading ~/.tenex/config.json")?;
    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    let id = identity::load_or_create(&edge, &agent, now_secs())?;
    // Our own fleet is auto-authorized: ensure this agent is on the allowlist.
    let _ = crate::acl::allow(&id.pubkey_hex(), &agent);
    let cwd = cwd.unwrap_or(std::env::current_dir()?);
    let project = project::resolve(&cwd);
    let session_id = session_id.unwrap_or_else(gen_session_id);

    let store = open_store()?;
    store.upsert_session(&crate::state::SessionRecord {
        session_id: session_id.clone(),
        agent_slug: agent.clone(),
        agent_pubkey: id.pubkey_hex(),
        project: project.clone(),
        host: cfg.host.clone(),
        child_pid: None,
        watch_pid,
        created_at: now_secs(),
        alive: true,
    })?;

    // Fork the detached engine: re-exec ourselves as `__run-session`.
    let exe = std::env::current_exe().context("locating own executable")?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("__run-session")
        .arg("--agent")
        .arg(&agent)
        .arg("--session-id")
        .arg(&session_id)
        .arg("--project")
        .arg(&project)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(pid) = watch_pid {
        command.arg("--watch-pid").arg(pid.to_string());
    }
    detach(&mut command);
    let child = command.spawn().context("forking background engine")?;

    // Record the engine pid so session-end can stop it; mark live immediately.
    let mut rec = store
        .get_session(&session_id)?
        .expect("just-written session");
    rec.child_pid = Some(child.id() as i32);
    store.upsert_session(&rec)?;
    store.touch_session(&session_id, now_secs())?;

    Ok(session_id)
}

#[cfg(unix)]
fn detach(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0); // own process group: survives terminal Ctrl-C / parent exit
}
#[cfg(not(unix))]
fn detach(_command: &mut std::process::Command) {}

// ── __run-session (the engine) ───────────────────────────────────────────────

async fn run_session(
    agent: String,
    session_id: String,
    project: String,
    watch_pid: Option<i32>,
) -> Result<()> {
    let cfg = Config::load()?;
    let edge = config::edge_home();
    let id = identity::load_or_create(&edge, &agent, now_secs())?;
    let _ = crate::acl::allow(&id.pubkey_hex(), &agent);

    let heartbeat = env_duration("TENEX_EDGE_HEARTBEAT_MS", Duration::from_secs(30));
    let obs_interval = env_duration("TENEX_EDGE_OBS_MS", Duration::from_secs(5));
    let status_ttl = Duration::from_secs(env_u64("TENEX_EDGE_STATUS_TTL_S", 90));
    // Turn-driven distillation cadence: first summary 30s into a turn (so quick
    // turns cost nothing), then refresh every 5m while it keeps running.
    let turn_first = Duration::from_secs(env_u64("TENEX_EDGE_TURN_FIRST_S", 30));
    let turn_repeat = Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 300));

    let params = EngineParams {
        agent_slug: agent,
        agent_pubkey: id.pubkey_hex(),
        keys: id.keys.clone(),
        project,
        session_id,
        host: cfg.host,
        owners: cfg.whitelisted_pubkeys,
        relays: cfg.relays,
        watch_pid,
        store_path: store_path(),
        heartbeat,
        obs_interval,
        status_ttl,
        turn_first,
        turn_repeat,
    };
    runtime::run_session(params).await
}

// ── session-end ──────────────────────────────────────────────────────────────

fn session_end(session: String) -> Result<()> {
    let store = open_store()?;
    if let Some(rec) = store.get_session(&session)? {
        if let Some(pid) = rec.child_pid {
            // SIGTERM -> engine publishes idle status and exits cleanly.
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            );
        }
        store.mark_session_dead(&session)?;
        eprintln!("session {session} ended");
    } else {
        eprintln!("no such session: {session}");
    }
    Ok(())
}

// ── send-message ─────────────────────────────────────────────────────────────

async fn send_message(recipient: String, message: String, session: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let store = open_store()?;
    let rec = resolve_session(&store, session)?;
    let edge = config::edge_home();
    let id = identity::load_or_create(&edge, &rec.agent_slug, now_secs())?;

    let (to_pubkey, target_session) = resolve_recipient(&store, &rec.project, &recipient)?;

    let mention = DomainEvent::Mention(Mention {
        from: AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: to_pubkey.clone(),
        project: rec.project.clone(),
        body: message,
        target_session: target_session.clone(),
    });

    let transport = Transport::connect(&cfg.relays, id.keys.clone()).await?;
    let codec = Kind1Codec;
    transport.publish_builder(codec.encode(&mention)?).await?;
    transport.shutdown().await;

    match target_session {
        Some(s) => println!(
            "mentioned {} (session {})",
            short_id(&to_pubkey),
            short_id(&s)
        ),
        None => println!("mentioned {}", short_id(&to_pubkey)),
    }
    Ok(())
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

fn resolve_recipient(
    store: &Store,
    my_project: &str,
    target: &str,
) -> Result<(String, Option<String>)> {
    // 1. slug@project
    if let Some((slug, proj)) = target.split_once('@') {
        let pk = store
            .resolve_agent_pubkey(slug, Some(proj))?
            .with_context(|| {
                format!("can't resolve {slug}@{proj} (no presence/profile seen yet)")
            })?;
        return Ok((pk, None));
    }
    // 2. raw hex pubkey
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok((target.to_string(), None));
    }
    // 3. session-id prefix (target a specific session of an agent) — check
    //    foreign peers first, then my own sessions on this machine.
    if target.len() >= 6 {
        if let Some(ps) = store.find_peer_session_by_prefix(target)? {
            return Ok((ps.pubkey, Some(ps.session_id)));
        }
        if let Some(s) = store.find_session_by_prefix(target)? {
            return Ok((s.agent_pubkey, Some(s.session_id)));
        }
    }
    // 4. bare agent slug in my project
    if let Some(pk) = store.resolve_agent_pubkey(target, Some(my_project))? {
        return Ok((pk, None));
    }
    bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
}

// ── who ──────────────────────────────────────────────────────────────────────

fn who(project: Option<String>, all: bool, all_projects: bool) -> Result<()> {
    let store = open_store()?;
    let current_project = if all_projects {
        None
    } else {
        Some(project.unwrap_or_else(|| {
            project::resolve(&std::env::current_dir().unwrap_or_default())
        }))
    };
    let snapshot = load_who_snapshot(&store, current_project.as_deref(), all, now_secs())?;
    print!("{}", render_who_once(&snapshot));
    Ok(())
}

fn who_live(project: Option<String>, all: bool, all_projects: bool, refresh: Duration) -> Result<()> {
    let refresh = refresh.max(Duration::from_millis(100));
    let store = open_store()?;
    let current_project = if all_projects {
        None
    } else {
        Some(project.unwrap_or_else(|| {
            project::resolve(&std::env::current_dir().unwrap_or_default())
        }))
    };
    let _terminal = LiveTerminal::enter()?;
    let mut next_draw = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_draw {
            let snapshot = load_who_snapshot(&store, current_project.as_deref(), all, now_secs())?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct OtherProjectSummary {
    project: String,
    agent_count: usize,
    about: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WhoSnapshot {
    project: String,
    all: bool,
    now: u64,
    rows: Vec<WhoRow>,
    other_projects: Vec<OtherProjectSummary>,
}


#[derive(Debug, Clone, PartialEq, Eq)]
struct WhoRow {
    source: WhoSource,
    fresh: bool,
    slug: String,
    project: String,
    status: String,
    host: String,
    session_id: String,
    age_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhoSource {
    Local,
    Peer,
}

fn load_who_snapshot(
    store: &Store,
    current_project: Option<&str>,
    all: bool,
    now: u64,
) -> Result<WhoSnapshot> {
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
    let mut other_counts: std::collections::BTreeMap<String, usize> =
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
            });
        } else {
            *other_counts.entry(s.project.clone()).or_default() += 1;
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
            });
        } else {
            *other_counts.entry(p.project.clone()).or_default() += 1;
        }
    }

    let other_projects = other_counts
        .into_iter()
        .map(|(project, agent_count)| {
            let about = store.get_project_meta(&project).ok().flatten();
            OtherProjectSummary {
                project,
                agent_count,
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

fn render_who_once(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();

    let scope = if snapshot.project == "*" {
        "any project".to_string()
    } else {
        snapshot.project.clone()
    };
    if snapshot.rows.is_empty() {
        let _ = writeln!(
            out,
            "(no live agents in {}{})",
            scope,
            if snapshot.all {
                ""
            } else {
                " — start a session, or run with --all to include stale"
            }
        );
    } else {
        let _ = writeln!(out, "{}", "agents:".bold());
        for row in &snapshot.rows {
            let dot = if row.fresh {
                "●".green().to_string()
            } else {
                "○".dimmed().to_string()
            };
            let status = render_status_colored(&row.status);
            match row.source {
                WhoSource::Local => {
                    let _ = writeln!(
                        out,
                        "  {dot} {}@{}{}  {}  session {}  {}",
                        row.slug.cyan(),
                        slugify_host(&row.host),
                        status,
                        row.project.dimmed(),
                        short_id(&row.session_id).yellow(),
                        "(this machine)".dimmed()
                    );
                }
                WhoSource::Peer => {
                    let age = row.age_secs.unwrap_or(0);
                    let _ = writeln!(
                        out,
                        "  {dot} {}@{}{}  {}  session {}  ({}s ago)",
                        row.slug.cyan(),
                        slugify_host(&row.host),
                        status,
                        row.project.dimmed(),
                        short_id(&row.session_id).yellow(),
                        age
                    );
                }
            }
        }
    }

    if snapshot.project != "*" && !snapshot.other_projects.is_empty() {
        let total: usize = snapshot.other_projects.iter().map(|o| o.agent_count).sum();
        let _ = writeln!(out);
        let _ = writeln!(out, "{} other agent(s) in other projects:", total);
        for op in &snapshot.other_projects {
            match &op.about {
                Some(about) if !about.is_empty() => {
                    let _ = writeln!(out, "  * {} — {}", op.project, about);
                }
                _ => {
                    let _ = writeln!(out, "  * {}", op.project);
                }
            }
        }
    }

    out
}

fn render_status_colored(status: &str) -> String {
    if status.trim().is_empty() {
        format!(" — {}", "idle".dimmed())
    } else {
        format!(" — {status}")
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
    write!(stdout, "{screen}")?;
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
                995,
            )
            .unwrap();
        store
            .set_agent_status("pk-reviewer", "proj", "reviewing the patch", 995)
            .unwrap();

        let snapshot = load_who_snapshot(&store, Some("proj"), false, 1_000).unwrap();

        assert_eq!(snapshot.rows.len(), 2);
        assert!(snapshot
            .rows
            .iter()
            .any(|r| r.source == WhoSource::Local && r.slug == "coder"));
        assert!(snapshot
            .rows
            .iter()
            .any(|r| r.source == WhoSource::Peer && r.slug == "reviewer"));
        assert!(!snapshot
            .rows
            .iter()
            .any(|r| r.source == WhoSource::Peer && r.session_id == "local-session"));

        let once = render_who_once(&snapshot);
        assert!(once.contains("@laptop"));
        assert!(once.contains("proj"));
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
            }],
            other_projects: vec![],
        };

        // --live uses render_who_once: same content, plus a dim quit-hint footer.
        let once = render_who_once(&snapshot);
        assert!(once.contains("reviewer"));
        assert!(once.contains("reviewing the patch"));
    }

}

// ── acl (owner-scoped agent authorization) ───────────────────────────────────

async fn acl(action: Option<AclAction>) -> Result<()> {
    let store = open_store()?;
    match action {
        Some(AclAction::Allow { target }) => {
            let (pk, slug) = resolve_acl_target(&store, &target)?;
            crate::acl::allow(&pk, &slug)?;
            store.remove_pending_agent(&pk).ok();
            println!("authorized {} ({})", slug.cyan(), short_id(&pk));
        }
        Some(AclAction::Block { target }) => {
            let (pk, slug) = resolve_acl_target(&store, &target)?;
            crate::acl::block(&pk, &slug)?;
            store.remove_pending_agent(&pk).ok();
            println!("blocked {} ({})", slug, short_id(&pk));
        }
        Some(AclAction::List) | None => {
            let pending = store.list_pending_agents()?;
            println!(
                "{}",
                "pending (claim you as owner, awaiting your decision):".bold()
            );
            if pending.is_empty() {
                println!("  (none)");
            } else {
                for p in &pending {
                    println!(
                        "  {} {}  ({})  host {}",
                        "?".yellow(),
                        p.slug.cyan(),
                        short_id(&p.pubkey),
                        p.host.dimmed()
                    );
                }
                println!(
                    "\n  allow:  tenex-edge acl allow <slug|pubkey>\n  block:  tenex-edge acl block <slug|pubkey>"
                );
            }
            let allowed = crate::acl::allowed();
            let blocked = crate::acl::blocked();
            println!(
                "\n{} {} authorized, {} blocked",
                "acl:".bold(),
                allowed.len(),
                blocked.len()
            );
        }
    }
    Ok(())
}

/// Resolve an `acl` target (pubkey, or a pending-agent slug) to (pubkey, slug).
fn resolve_acl_target(store: &Store, target: &str) -> Result<(String, String)> {
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        let slug = store
            .list_pending_agents()?
            .into_iter()
            .find(|p| p.pubkey == target)
            .map(|p| p.slug)
            .unwrap_or_else(|| "agent".to_string());
        return Ok((target.to_string(), slug));
    }
    // else treat as a pending-agent slug
    let m = store
        .list_pending_agents()?
        .into_iter()
        .find(|p| p.slug == target);
    match m {
        Some(p) => Ok((p.pubkey, p.slug)),
        None => bail!("no pending agent named {target:?}; use a pubkey or `tenex-edge acl list`"),
    }
}

// ── inbox ────────────────────────────────────────────────────────────────────

async fn inbox(session: Option<String>) -> Result<()> {
    let store = open_store()?;
    let rec = resolve_session(&store, session)?;
    // Self-fetch: pull recent stored mentions addressed to me straight from the
    // relay, so receive works even in one-shot sessions where the background
    // engine hasn't caught up yet. Best-effort; never blocks the drain.
    let _ = fetch_mentions_into_inbox(&store, &rec).await;
    let rows = store.drain_inbox(&rec.session_id)?;
    for r in &rows {
        println!("[mention from {}@{}] {}", r.from_slug, r.project, r.body);
        // Mark seen per-agent so it never resurfaces in a later session.
        store
            .mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
            .ok();
    }
    // Surface agents awaiting the human's authorization decision, so the agent
    // can tell its human (the injection hook prints this into context).
    let pending = store.list_pending_agents().unwrap_or_default();
    if !pending.is_empty() {
        let names: Vec<String> = pending
            .iter()
            .map(|p| format!("{} ({})", p.slug, short_id(&p.pubkey)))
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

async fn fetch_mentions_into_inbox(store: &Store, rec: &crate::state::SessionRecord) -> Result<()> {
    use nostr_sdk::prelude::{Filter, Kind, PublicKey};
    let cfg = Config::load()?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let me = id.pubkey_hex();
    let pk = PublicKey::from_hex(&me)?;
    let transport = Transport::connect(&cfg.relays, id.keys.clone()).await?;
    let codec = Kind1Codec;
    let filter = Filter::new().kind(Kind::from(1u16)).pubkey(pk).limit(50);
    if let Ok(events) = transport.fetch(filter, Duration::from_secs(3)).await {
        for ev in events {
            if let Some(DomainEvent::Mention(m)) = codec.decode(&ev) {
                if m.to_pubkey != me {
                    continue;
                }
                // Skip mentions this agent already received in a prior session.
                if store.is_mention_seen(&me, &ev.id.to_hex()).unwrap_or(false) {
                    continue;
                }
                // Deliver to this session if it's the target, or if untargeted.
                let deliver = m
                    .target_session
                    .as_deref()
                    .map(|t| t == rec.session_id)
                    .unwrap_or(true);
                if deliver {
                    let _ = store.enqueue_mention(&crate::state::InboxRow {
                        mention_event_id: ev.id.to_hex(),
                        target_session: rec.session_id.clone(),
                        from_pubkey: m.from.pubkey.clone(),
                        from_slug: m.from.slug.clone(),
                        project: m.project.clone(),
                        body: m.body.clone(),
                        created_at: now_secs(),
                    });
                }
            }
        }
    }
    transport.shutdown().await;
    Ok(())
}

// ── wait-for-mention ─────────────────────────────────────────────────────────

async fn wait_for_mention(session: Option<String>, timeout: u64) -> Result<()> {
    let store = open_store()?;
    let rec = resolve_session(&store, session)?;
    // Self-fetch handles the engine warmup race (same as inbox).
    let _ = fetch_mentions_into_inbox(&store, &rec).await;

    let deadline = if timeout > 0 {
        Some(std::time::Instant::now() + Duration::from_secs(timeout))
    } else {
        None
    };

    loop {
        let rows = store.drain_inbox(&rec.session_id)?;
        if !rows.is_empty() {
            for r in &rows {
                println!("[mention from {}@{}] {}", r.from_slug, r.project, r.body);
                store
                    .mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                    .ok();
            }
            println!("[tenex-edge] Run `tenex-edge wait-for-mention` with run_in_background=true to receive the next mention.");
            return Ok(());
        }
        if deadline
            .map(|d| std::time::Instant::now() >= d)
            .unwrap_or(false)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ── turn-start / turn-check / turn-end ───────────────────────────────────────

async fn turn_start(session: String, transcript: Option<String>, json_out: bool) -> Result<()> {
    if session.is_empty() {
        return Ok(());
    }
    let store = open_store()?;

    let (_, prev_turn_started_at) = store.get_turn_state(&session)?;
    let first_turn = prev_turn_started_at == 0;

    store.mark_turn_start(&session, now_secs())?;
    if let Some(ref path) = transcript {
        if !path.is_empty() {
            store.set_session_transcript(&session, path).ok();
        }
    }

    let rec = match store.get_session(&session)? {
        Some(r) => r,
        None => return Ok(()), // race with session-start, nothing to inject yet
    };

    let _ = fetch_mentions_into_inbox(&store, &rec).await;

    let mut blocks: Vec<String> = Vec::new();

    if first_turn {
        blocks.push(
            "[tenex-edge] You are connected to the tenex-edge agent fabric. \
             You can run `tenex-edge who`, `tenex-edge inbox`, and \
             `tenex-edge send-message --recipient <agent|session-id> --message \"...\"`. \
             If the user asks you to message/contact/tell another agent, run `tenex-edge send-message`; \
             do not say you cannot send messages from here. Run `tenex-edge wait-for-mention` \
             with run_in_background=true so you are woken when a mention arrives. \
             Re-run it each time one is received."
                .to_string(),
        );
    }

    // Drain inbox (authoritative delivery; turn_check only peeks).
    let inbox_rows = store.drain_inbox(&rec.session_id)?;
    if !inbox_rows.is_empty() {
        let mut text = String::from("Messages from other agents (tenex-edge):");
        for r in &inbox_rows {
            let _ = write!(text, "\n[mention from {}@{}] {}", r.from_slug, r.project, r.body);
            store
                .mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                .ok();
        }
        blocks.push(text);
    }

    // Pending ACL agents (unknown agents claiming this owner).
    let pending = store.list_pending_agents().unwrap_or_default();
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
    if first_turn {
        let snapshot = load_who_snapshot(&store, Some(&rec.project), false, now_secs())?;
        if !snapshot.rows.is_empty() {
            let who_text = render_who_plain(&snapshot);
            blocks.push(format!(
                "tenex-edge fabric — agents you can message. To send, run \
                 `tenex-edge send-message --recipient <agent|session-id> --message \"...\"`:\n{}",
                who_text.trim_end()
            ));
        }
    } else {
        // Only surface new arrivals and status changes since the last turn began.
        let fresh_since = now_secs().saturating_sub(PEER_FRESH_SECS);
        let new_peers = store
            .list_new_peer_sessions(prev_turn_started_at, fresh_since, Some(&rec.project))
            .unwrap_or_default();
        let status_changes = store
            .list_status_changes_since(prev_turn_started_at, Some(&rec.project))
            .unwrap_or_default();

        let mut delta: Vec<String> = Vec::new();
        for p in &new_peers {
            let age = now_secs().saturating_sub(p.last_seen);
            delta.push(format!(
                "  ● {}@{} joined  {}  session {}  ({age}s ago)",
                p.slug,
                slugify_host(&p.host),
                p.project,
                short_id(&p.session_id),
            ));
        }
        for (slug, project, text) in &status_changes {
            delta.push(format!("  ↻ {slug}@{project} — {text}"));
        }
        if !delta.is_empty() {
            blocks.push(format!(
                "tenex-edge fabric — changes since your last turn:\n{}",
                delta.join("\n")
            ));
        }
    }

    if !blocks.is_empty() {
        emit_context(&blocks.join("\n\n"), json_out);
    }
    Ok(())
}

/// Mid-turn inbox check for PostToolUse hooks. Read-only: only peeks the inbox
/// without draining. No writes to state.db.
fn turn_check(session: Option<String>, json_out: bool) -> Result<()> {
    let store = open_store()?;
    let rec = resolve_session(&store, session)?;

    let rows = store.peek_inbox(&rec.session_id)?;
    if rows.is_empty() {
        return Ok(());
    }

    let mut text = String::from("[tenex-edge] Message(s) arrived while you were working:");
    for r in &rows {
        let _ = write!(text, "\n[mention from {}@{}] {}", r.from_slug, r.project, r.body);
    }
    emit_context(&text, json_out);
    Ok(())
}

fn render_who_plain(snapshot: &WhoSnapshot) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "agents:");
    for row in &snapshot.rows {
        let dot = if row.fresh { "●" } else { "○" };
        let status = status_plain(&row.status);
        match row.source {
            WhoSource::Local => {
                let _ = writeln!(
                    out,
                    "  {dot} {}@{} — {status}  {}  session {}  (this machine)",
                    row.slug,
                    slugify_host(&row.host),
                    row.project,
                    short_id(&row.session_id),
                );
            }
            WhoSource::Peer => {
                let age = row.age_secs.unwrap_or(0);
                let _ = writeln!(
                    out,
                    "  {dot} {}@{} — {status}  {}  session {}  ({age}s ago)",
                    row.slug,
                    slugify_host(&row.host),
                    row.project,
                    short_id(&row.session_id),
                );
            }
        }
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
    let store = open_store()?;
    store.mark_turn_end(&session)?;
    Ok(())
}

// ── doctor ───────────────────────────────────────────────────────────────────

async fn doctor() -> Result<()> {
    use nostr_sdk::prelude::{Alphabet, EventBuilder, Filter, Keys, Kind, SingleLetterTag, Tag};
    let cfg = Config::load()?;
    println!("relays: {:?}", cfg.relays);
    let keys = Keys::generate();
    println!("probe pubkey: {}", keys.public_key().to_hex());
    let t = format!("te-doctor-{}", now_secs());

    let transport = Transport::connect(&cfg.relays, keys).await?;
    let builder = EventBuilder::new(Kind::from(1u16), format!("tenex-edge doctor {t}"))
        .tags([Tag::parse(["h", &t])?]);
    match transport.publish_builder(builder).await {
        Ok(id) => println!("publish: OK ({})", short_id(&id.to_hex())),
        Err(e) => println!("publish: ERR {e:#}"),
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    let f = Filter::new()
        .kind(Kind::from(1u16))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &t)
        .limit(5);
    match transport.fetch(f, Duration::from_secs(5)).await {
        Ok(evs) => println!("read-back: {} event(s) with #h={t}", evs.len()),
        Err(e) => println!("read-back: ERR {e:#}"),
    }
    transport.shutdown().await;
    Ok(())
}

// ── tail ─────────────────────────────────────────────────────────────────────

async fn tail(project: Option<String>) -> Result<()> {
    let cfg = Config::load()?;
    let reader = Transport::connect(&cfg.relays, nostr_sdk::prelude::Keys::generate()).await?;
    let codec = Kind1Codec;
    let scope = SubScope {
        authors: Vec::new(),
        project: project.clone(),
        mentions_to: None,
        owners: Vec::new(),
    };
    reader.subscribe(codec.filters(&scope)).await?;
    let mut notifications = reader.notifications();

    let scope_label = project.as_deref().unwrap_or("*");
    eprintln!(
        "{} tailing project {} … (Ctrl-C to stop)",
        "tenex-edge".bold(),
        scope_label.cyan()
    );

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            n = notifications.recv() => match n {
                Ok(RelayPoolNotification::Event { event, .. }) => {
                    if let Some(de) = codec.decode(&event) {
                        println!("{}", render(&de));
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(_) => {}
            }
        }
    }
    reader.shutdown().await;
    Ok(())
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

fn env_duration(key: &str, default: Duration) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
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
}

static HOOK_HOSTS: &[HostDef] = &[
    HostDef {
        name: "claude-code",
        agent_slug: "claude",
        session_id_fields: &["session_id"],
        transcript_field: Some("transcript_path"),
        output_format: HookOutputFormat::PlainText,
        pid_search: None,
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
            if sid.is_empty() {
                return Ok(());
            }
            let watch_pid = host.pid_search.and_then(find_ancestor_pid);
            // Discard returned session id — hooks receive it from the harness.
            session_start_inner(agent_slug, Some(sid), Some(cwd), watch_pid)?;
        }
        "session-end" => {
            if !sid.is_empty() {
                session_end(sid)?;
            }
        }
        "user-prompt-submit" => {
            turn_start(sid, transcript, json_out).await?;
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
