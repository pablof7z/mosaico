//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{
    dirty_label, format_local_datetime, now_secs, pubkey_short, relative_time, session_short_code,
    slugify_host, SessionId,
};
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

mod admin;
mod hooks;
mod messaging;
mod turn;
mod who;

pub use admin::render_fabric;
pub use messaging::{format_envelope, mention_short_id, EnvelopeView};
pub use turn::{assemble_turn_check_context, assemble_turn_start_context};
pub use who::load_who_snapshot;

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
    /// Read your messages (bare `inbox`), or `send` / `reply` to other agents.
    ///
    /// Bare `inbox` prints + drains pending mentions for a session — used by the
    /// opencode injection path and as a manual "check my messages" command.
    /// (Claude Code and Codex drain via the `hook --type user-prompt-submit` path.)
    Inbox {
        #[command(subcommand)]
        action: Option<InboxAction>,
        /// Session id; if omitted, resolved from the current directory.
        #[arg(long, global = true)]
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
    /// Publish a long-form proposal (kind:30023) from this agent's session.
    Propose {
        /// Proposal title.
        #[arg(long)]
        title: String,
        /// Proposal body (Markdown). Use "-" or omit to read from stdin.
        #[arg(long = "message", value_name = "BODY")]
        message: Option<String>,
        /// Event id of the conversation this proposal belongs to (becomes an "e" root tag).
        #[arg(long = "thread", value_name = "EVENT_ID")]
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
enum InboxAction {
    /// Send a message to another agent or a specific session.
    Send {
        /// Recipient: session-id (or prefix), agent slug, slug@project, or hex pubkey.
        #[arg(long = "to", value_name = "RECIPIENT")]
        to: String,
        /// One-line subject ("what this is about").
        #[arg(long)]
        subject: Option<String>,
        /// Message body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
    },
    /// Reply to a message by its ID (the `ID:` shown on each message you receive).
    Reply {
        /// The ID shown on the message you're replying to.
        #[arg(long)]
        id: String,
        /// Subject; defaults to "Re: <original subject>".
        #[arg(long)]
        subject: Option<String>,
        /// Reply body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
    },
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
        Cmd::Who {
            project,
            all,
            all_projects,
            live,
            refresh_ms,
        } => {
            if live {
                who::who_live(
                    project,
                    all,
                    all_projects,
                    Duration::from_millis(refresh_ms.max(100)),
                )
            } else {
                who::who(project, all, all_projects)
            }
        }
        Cmd::Propose {
            title,
            message,
            thread_id,
            d,
            session,
        } => {
            let body = messaging::resolve_send_message_body(message)?;
            messaging::propose(title, body, thread_id, d, session).await
        }
        Cmd::Acl { action } => admin::acl(action).await,
        Cmd::Tail { project } => admin::tail(project).await,
        Cmd::Inbox { action, session } => match action {
            None => messaging::inbox(session).await,
            Some(InboxAction::Send {
                to,
                subject,
                message,
                message_flag,
            }) => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::inbox_send(to, subject, message, session).await
            }
            Some(InboxAction::Reply {
                id,
                subject,
                message,
                message_flag,
            }) => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::inbox_reply(id, subject, message, session).await
            }
        },
        Cmd::WaitForMention { session, timeout } => {
            messaging::wait_for_mention(session, timeout).await
        }
        Cmd::Project { action } => admin::project(action).await,
        Cmd::Doctor => admin::doctor().await,
        Cmd::Hook { host, hook_type } => hooks::hook_run(host, hook_type).await,
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

async fn daemon_call_async(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}
