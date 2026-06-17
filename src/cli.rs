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
pub mod command_forensics;
mod debug;
mod hooks;
mod messaging;
mod statusline;
mod tmux_cli;
mod turn;
mod who;

pub use admin::render_fabric;
#[cfg(test)]
use admin::{parse_since, render_tail_event};
pub(crate) use messaging::row_envelope;
pub use messaging::{format_envelope, mention_short_id, EnvelopeView};
pub(crate) use turn::render_chat_block;
pub use turn::{assemble_turn_check_context, assemble_turn_start_context};
pub use who::load_who_snapshot;

pub(crate) fn select_agent_env(active: Option<String>, fallback: Option<String>) -> Option<String> {
    active
        .filter(|s| !s.is_empty())
        .or_else(|| fallback.filter(|s| !s.is_empty()))
}

pub(crate) fn agent_env_slug() -> Option<String> {
    select_agent_env(
        std::env::var("TENEX_EDGE_AGENT").ok(),
        std::env::var("TENEX_EDGE_AGENT_FALLBACK").ok(),
    )
}

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
        /// Show only these categories (comma-separated: msg,sync,turn,stat,join,leave,sess,proj,profile).
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
    /// Write or read NIP-29 project chat.
    Chat {
        #[command(subcommand)]
        action: ChatAction,
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
    /// Render the one-line fabric statusline for a host's status bar.
    /// Reads the harness's statusline JSON payload on stdin (for `session_id`),
    /// prints one line, and always exits 0 — fails open when the daemon is down
    /// (and never spawns one).
    Statusline {
        /// Session id; if omitted, taken from the stdin payload or resolved from cwd.
        #[arg(long)]
        session: Option<String>,
    },
    /// Connectivity check: publish a test note to the configured relays and read it back.
    Doctor,
    /// Local debugging tools for hook injection and command telemetry.
    Debug {
        #[command(subcommand)]
        action: DebugAction,
    },
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
    /// TMUX control-plane commands: status, inject pending messages, spawn agent, attach.
    /// With no subcommand, opens an interactive TUI.
    Tmux {
        #[command(subcommand)]
        action: Option<TmuxAction>,
        /// Run the bare TUI in popup mode: selecting a session switches the
        /// underlying tmux client and exits (closing the `display-popup`),
        /// instead of attaching inline. Used by the `M-t` quick-switcher.
        #[arg(long, hide = true)]
        popup: bool,
    },
    /// Launch an agent harness in a new tmux session, with tmux chrome hidden.
    Launch {
        /// Agent slug: "claude", "codex", "opencode", or a local custom agent.
        slug: String,
        /// Project slug; defaults to project resolved from current directory.
        #[arg(long)]
        project: Option<String>,
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
        /// Canonical thread id to reply into (encodes NIP-10 root e-tag on the
        /// published event so the recipient groups the reply into the same thread).
        /// Omit for a new root message.
        #[arg(long = "thread", value_name = "THREAD_ID")]
        thread_id: Option<String>,
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
enum ChatAction {
    /// Publish a project chat line. Reads body from arg, --message, or stdin.
    Write {
        /// Highlight a session in the project chat.
        #[arg(long)]
        mention: Option<String>,
        /// Message body. Positional, or via --message, or piped on stdin.
        #[arg(value_name = "MESSAGE")]
        message: Option<String>,
        #[arg(long = "message", value_name = "MESSAGE")]
        message_flag: Option<String>,
        /// My session id; if omitted, resolved from the current directory.
        #[arg(long)]
        session: Option<String>,
    },
    /// Read project chat history.
    Read {
        /// Only show messages after this time (unix timestamp or duration like "1h").
        #[arg(long)]
        since: Option<String>,
        /// Maximum messages to print.
        #[arg(long)]
        limit: Option<u64>,
        /// Skip this many messages after ordering/filtering.
        #[arg(long)]
        offset: Option<u64>,
        /// Page from the newest messages; output remains chronological.
        #[arg(long)]
        tail: bool,
        /// Keep the chat reader open and print new messages as they arrive.
        #[arg(long)]
        live: bool,
        /// Project slug; defaults to the project resolved from the current directory.
        #[arg(long, hide = true)]
        project: Option<String>,
    },
}

#[derive(Subcommand)]
enum TmuxAction {
    /// List registered tmux endpoints with liveness info.
    Status,
    /// Manually inject pending messages into a session's pane (debug).
    Send {
        /// Session id (or prefix) to inject.
        #[arg(long)]
        session: String,
    },
    /// Spawn a new tmux window running the given agent harness.
    Spawn {
        /// Agent slug: "claude", "codex", "opencode", …
        #[arg(long)]
        agent: String,
        /// Project slug; defaults to project resolved from current directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Exec into the tmux pane registered for a session.
    Attach {
        /// Session id (or prefix).
        #[arg(long)]
        session: String,
    },
    /// Resume a (typically dead) session: replay its harness in a new tmux
    /// window using the captured native resume token, then attach to it.
    Resume {
        /// Session id (or prefix) to resume.
        #[arg(long)]
        session: String,
    },
    /// Long-running sidebar process: list project sessions in a narrow pane,
    /// highlight the current session, and let the user switch between them.
    /// Normally started automatically by `ensure_sidebar`; can also be run
    /// manually with `tenex-edge tmux sidebar --session <id>`.
    Sidebar {
        /// The session this sidebar belongs to (highlighted as "current").
        /// If omitted, resolved at runtime from the tmux client session name.
        #[arg(long)]
        session: Option<String>,
        /// Project to filter by. If omitted, derived from the current session's
        /// live row in the daemon data.
        #[arg(long)]
        project: Option<String>,
    },
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

#[derive(Subcommand)]
enum DebugAction {
    /// Live TUI for hook injections and tenex-edge command invocations.
    HookTail {
        /// Filter panes/events to a project.
        #[arg(long)]
        project: Option<String>,
        /// Filter panes/events to a session id or short code.
        #[arg(long)]
        session: Option<String>,
        /// Maximum panes in the grid.
        #[arg(long, default_value = "6")]
        panes: usize,
        /// Refresh interval in milliseconds.
        #[arg(long, default_value = "1000")]
        refresh_ms: u64,
    },
}

pub async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
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
        Cmd::Threads { project, thread } => messaging::threads(project, thread).await,
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
        Cmd::Tail {
            project,
            agent,
            host,
            since,
            backfill,
            only,
            exclude,
            include,
            all,
            compact,
            relative,
            no_emoji,
            no_color,
            json,
            no_follow,
            live,
        } => {
            admin::tail(admin::TailOpts {
                project,
                agent,
                host,
                since,
                backfill,
                only,
                exclude,
                include,
                all,
                compact,
                relative,
                no_emoji,
                no_color,
                json,
                no_follow,
                live,
            })
            .await
        }
        Cmd::Inbox { action, session } => match action {
            None => messaging::inbox(session).await,
            Some(InboxAction::Send {
                to,
                subject,
                message,
                message_flag,
                thread_id,
            }) => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::inbox_send(to, subject, message, session, thread_id).await
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
        Cmd::Chat { action } => match action {
            ChatAction::Write {
                mention,
                message,
                message_flag,
                session,
            } => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::chat_write(message, mention, session).await
            }
            ChatAction::Read {
                since,
                limit,
                offset,
                tail,
                live,
                project,
            } => messaging::chat_read(since, limit, offset, tail, live, project).await,
        },
        Cmd::WaitForMention { session, timeout } => {
            messaging::wait_for_mention(session, timeout).await
        }
        Cmd::Statusline { session } => statusline::statusline(session),
        Cmd::Project { action } => admin::project(action).await,
        Cmd::Doctor => admin::doctor().await,
        Cmd::Debug { action } => match action {
            DebugAction::HookTail {
                project,
                session,
                panes,
                refresh_ms,
            } => debug::hook_tail(debug::HookTailOpts {
                project,
                session,
                panes,
                refresh: Duration::from_millis(refresh_ms.max(100)),
            }),
        },
        Cmd::Hook { host, hook_type } => hooks::hook_run(host, hook_type).await,
        Cmd::Tmux { action, popup } => match action {
            Some(action) => tmux_cli::tmux_run(action).await,
            None => tmux_cli::tmux_tui(popup),
        },
        Cmd::Launch { slug, project } => tmux_cli::launch(slug, project).await,
        Cmd::Daemon => crate::daemon::server::run().await,
    }
}

/// A peer is "live" only while heartbeats keep it fresh (3× the default 30s tick).
const PEER_FRESH_SECS: u64 = 90;

// Session resolution, session-id generation, recipient resolution, and the
// store live INSIDE the daemon now (it is the sole writer). The CLI verbs below
// are thin clients that forward to it over the UDS.

// ── session-end ──────────────────────────────────────────────────────────────

pub(super) fn session_end(session: String) -> Result<()> {
    let v = crate::daemon::blocking::call("session_end", serde_json::json!({"session": session}))?;
    if v["ended"].as_bool().unwrap_or(false) {
        eprintln!("session {session} ended");
    } else {
        eprintln!("no such session: {session}");
    }
    Ok(())
}

/// Async daemon call helper for `async fn` verbs (uses the async client; we are
/// inside the tokio runtime so we must NOT block_on a sync client here).
pub(super) async fn daemon_call_async(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

// ── freeze tests — turn-start / turn-check context assembly ─────────────────

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::session::{Harness, SessionObservation};
    use crate::state::{InboxRow, SessionRecord, Store};
    use std::sync::Mutex;

    /// Register a local session into `session_state` (daemon mints the canonical
    /// id) and return it.
    fn register_local(
        store: &Store,
        slug: &str,
        pubkey: &str,
        harness_sid: &str,
        ts: u64,
    ) -> String {
        let obs = SessionObservation {
            agent_slug: slug.to_string(),
            agent_pubkey: pubkey.to_string(),
            project: "proj".to_string(),
            host: "laptop".to_string(),
            rel_cwd: String::new(),
            harness: Harness::ClaudeCode,
            harness_session_id: Some(harness_sid.to_string()),
            resume_id: None,
            tmux_pane: None,
            watch_pid: None,
            observed_at: ts,
        };
        store
            .register_or_reassert_session(&obs)
            .unwrap()
            .session_id
            .as_str()
            .to_string()
    }

    /// Register a busy local session carrying a distilled title + activity line.
    /// Appears at `reg_ts` (so a cursor after it sees a *change*, not an appear)
    /// and the distill lands at `change_ts`.
    fn register_busy(
        store: &Store,
        slug: &str,
        pubkey: &str,
        harness_sid: &str,
        title: &str,
        activity: &str,
        reg_ts: u64,
        change_ts: u64,
    ) -> String {
        let id = register_local(store, slug, pubkey, harness_sid, reg_ts);
        let snap = store.start_turn(&id, change_ts).unwrap().unwrap();
        store
            .apply_distill_result(
                &id,
                snap.turn_id,
                snap.state_version,
                title,
                activity,
                change_ts,
            )
            .unwrap()
            .unwrap();
        id
    }

    /// Register a local session that opened and then finished a turn, so it is
    /// idle but retains its title. Appears at `reg_ts`; the busy→idle change
    /// lands at `change_ts`.
    fn register_idle(
        store: &Store,
        slug: &str,
        pubkey: &str,
        harness_sid: &str,
        title: &str,
        reg_ts: u64,
        change_ts: u64,
    ) -> String {
        let id = register_local(store, slug, pubkey, harness_sid, reg_ts);
        let snap = store.start_turn(&id, change_ts).unwrap().unwrap();
        store
            .seed_title_if_empty(&id, snap.turn_id, title, change_ts)
            .unwrap()
            .unwrap();
        store.end_turn(&id, change_ts).unwrap().unwrap();
        id
    }

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
            subject: String::new(),
            branch: String::new(),
            commit: String::new(),
            dirty: 0,
            host: String::new(),
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
        store
            .enqueue_mention(&inbox_row("sess-freeze-1", "evt-c1"))
            .unwrap();

        let m = Mutex::new(store);

        // Non-first turn (prev != 0) → no intro block; no peers → no fabric block.
        // Only the inbox mention block should be present.
        let ctx = assemble_turn_start_context(&m, &rec, /* prev_turn_started_at */ 1);
        let text = ctx.expect("FREEZE: turn_start must return Some when inbox has rows");

        assert!(
            text.contains("Messages from other agents (tenex-edge)"),
            "FREEZE: mention section header must be present; got: {text:?}"
        );
        assert!(
            text.contains("From: sender@proj"),
            "envelope From line must be present; got: {text:?}"
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
        store
            .enqueue_mention(&inbox_row("sess-freeze-3", "evt-c3"))
            .unwrap();
        let m = Mutex::new(store);

        // turn_check peeks: returns Some with the mention line. delta_since=None
        // isolates the inbox-peek path (no sibling-delta query).
        let ctx =
            assemble_turn_check_context(&m, &test_session("sess-freeze-3"), "laptop", None, 200);
        let text =
            ctx.expect("FREEZE: turn_check must return Some when inbox has undelivered rows");
        assert!(
            text.contains("[tenex-edge] Message(s) arrived while you were working:"),
            "FREEZE: turn_check header must be present; got: {text:?}"
        );
        assert!(
            text.contains("From: sender@proj"),
            "turn_check must render the envelope From line; got: {text:?}"
        );
        assert!(
            text.contains("ID: evt-c3"),
            "turn_check envelope must carry the reply ID; got: {text:?}"
        );

        // The row is still in the store (peek, not drain): drain_inbox now consumes it.
        let g = m.lock().unwrap();
        let drained = g.drain_inbox("sess-freeze-3").unwrap();
        assert_eq!(
            drained.len(),
            1,
            "FREEZE: row must survive turn_check peek and still be drainable"
        );
    }

    /// FREEZE C4: assemble_turn_check_context returns None when inbox is empty.
    #[test]
    fn freeze_turn_check_context_returns_none_when_inbox_empty() {
        let store = Store::open_memory().unwrap();
        let m = Mutex::new(store);
        let ctx =
            assemble_turn_check_context(&m, &test_session("sess-no-rows"), "laptop", None, 200);
        assert!(
            ctx.is_none(),
            "FREEZE: turn_check with empty inbox must return None"
        );
    }

    /// Mid-turn delta: a sibling session's status change in the same project is
    /// surfaced with its activity line; the viewer's own row is excluded.
    #[test]
    fn turn_check_delta_shows_siblings_with_activity_excludes_self() {
        let store = Store::open_memory().unwrap();
        store.upsert_profile("pk-sib", "sib", "laptop", 1).unwrap();
        // Sibling registered before the cursor (10), then changed after it (180)
        // and is still live at now=200 → surfaces as a Changed delta.
        let sib_id = register_busy(
            &store,
            "sib",
            "pk-sib",
            "sess-sib",
            "Refactor tmux",
            "editing hooks.rs",
            10,
            180,
        );
        // The viewer's own session also changed — must NOT echo back.
        let me_id = register_busy(
            &store,
            "coder",
            "pk-coder",
            "sess-me",
            "My own work",
            "typing",
            10,
            180,
        );
        let m = Mutex::new(store);

        let text = assemble_turn_check_context(&m, &test_session(&me_id), "laptop", Some(50), 200)
            .expect("delta block expected when a sibling changed");
        assert!(
            text.contains("changes since your last check"),
            "delta header expected; got: {text:?}"
        );
        // Changed renders `↻ sib@proj [session <code>] — Refactor tmux — editing hooks.rs`.
        assert!(
            text.contains("sib@proj") && text.contains("Refactor tmux — editing hooks.rs"),
            "sibling title+activity expected; got: {text:?}"
        );
        assert!(
            !text.contains("My own work"),
            "viewer's own status must be excluded; got: {text:?}"
        );
        // The session must render as the targetable short code (matching `who`),
        // never the raw id — otherwise it can't be copied into `send --to`.
        assert!(
            text.contains(&crate::util::session_short_code(&sib_id)),
            "session must render as short code; got: {text:?}"
        );
        assert!(
            !text.contains(sib_id.as_str()),
            "raw session id must not leak; got: {text:?}"
        );
    }

    /// Mid-turn delta: a sibling that went idle renders with the `· idle` marker
    /// so peers can see when someone stopped working.
    #[test]
    fn turn_check_delta_shows_idle_transition() {
        let store = Store::open_memory().unwrap();
        store.upsert_profile("pk-sib", "sib", "laptop", 1).unwrap();
        // Sibling appeared before the cursor (10), then opened+finished a turn at
        // 180 → idle, title retained, still live at now=200 → Changed delta.
        register_idle(
            &store,
            "sib",
            "pk-sib",
            "sess-sib",
            "Refactor tmux",
            10,
            180,
        );
        let m = Mutex::new(store);

        let text =
            assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200)
                .expect("delta block expected for idle transition");
        assert!(
            text.contains("Refactor tmux · idle"),
            "idle marker expected; got: {text:?}"
        );
    }

    /// Repeated idle/end observations are liveness refreshes, not user-visible
    /// status changes. They must not re-emit the same `title · idle` line.
    #[test]
    fn turn_check_delta_suppresses_repeated_idle_noop() {
        let store = Store::open_memory().unwrap();
        store.upsert_profile("pk-sib", "sib", "laptop", 1).unwrap();
        let sib_id = register_idle(&store, "sib", "pk-sib", "sess-sib", "Refactor tmux", 10, 20);
        store.end_turn(&sib_id, 180).unwrap().unwrap();
        let m = Mutex::new(store);

        let text =
            assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200);
        assert!(
            text.is_none(),
            "unchanged idle session must not be emitted again; got: {text:?}"
        );
    }

    /// Repeated session-start/reassert observations refresh liveness and tmux
    /// endpoint aliases, but identical public state is not a status delta.
    #[test]
    fn turn_check_delta_suppresses_identical_session_reassert() {
        let store = Store::open_memory().unwrap();
        store.upsert_profile("pk-sib", "sib", "laptop", 1).unwrap();
        register_local(&store, "sib", "pk-sib", "sess-sib", 10);
        register_local(&store, "sib", "pk-sib", "sess-sib", 180);
        let m = Mutex::new(store);

        let text =
            assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200);
        assert!(
            text.is_none(),
            "identical session reassert must not be emitted as a change; got: {text:?}"
        );
    }

    /// `delta_since = None` (rate-limited / not mid-turn) suppresses the sibling
    /// delta entirely, even when a sibling just changed.
    #[test]
    fn turn_check_delta_suppressed_when_not_due() {
        let store = Store::open_memory().unwrap();
        store.upsert_profile("pk-sib", "sib", "laptop", 1).unwrap();
        register_busy(
            &store,
            "sib",
            "pk-sib",
            "sess-sib",
            "Refactor tmux",
            "",
            10,
            180,
        );
        let m = Mutex::new(store);

        let ctx = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", None, 200);
        assert!(
            ctx.is_none(),
            "no delta and no inbox → None when not due; got: {ctx:?}"
        );
    }

    fn view<'a>() -> EnvelopeView<'a> {
        EnvelopeView {
            from_slug: "codex",
            project: "tenex-edge",
            from_session: "sender-session-id",
            host: "",
            self_host: "my-box",
            subject: "NIP-29 group creation failing",
            branch: "features/oauth",
            commit: "a1b2c3d",
            dirty: 0,
            id: "01234567",
            sent_at: 1_000,
            now: 1_180, // +3 min
            body: "can you take a look?",
        }
    }

    #[test]
    fn envelope_has_email_like_headers_then_body() {
        let out = format_envelope(&view());
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines[0],
            format!(
                "From: codex@tenex-edge [session {}]",
                session_short_code("sender-session-id")
            )
        );
        assert!(lines[1].starts_with("Date: ") && lines[1].ends_with("(3 min ago)"));
        assert_eq!(lines[2], "Subject: NIP-29 group creation failing");
        assert_eq!(lines[3], "Branch: features/oauth (a1b2c3d)");
        assert_eq!(lines[4], "ID: 01234567");
        assert_eq!(lines[5], "--");
        assert_eq!(lines[6], "can you take a look?");
    }

    #[test]
    fn dirty_count_and_remote_host_annotate() {
        let mut v = view();
        v.dirty = 1;
        v.host = "prod-01.example.com";
        let out = format_envelope(&v);
        assert!(out.contains("[remote: prod-01.example.com]"));
        assert!(out.contains("Branch: features/oauth (a1b2c3d) [1 file dirty]"));
        v.dirty = 3;
        assert!(format_envelope(&v).contains("[3 files dirty]"));
    }

    #[test]
    fn subject_and_branch_lines_omitted_when_empty() {
        let mut v = view();
        v.subject = "";
        v.branch = "";
        let out = format_envelope(&v);
        assert!(!out.contains("Subject:"));
        assert!(!out.contains("Branch:"));
        // Same-host sender → no remote annotation.
        assert!(!out.contains("remote:"));
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

    #[test]
    fn agent_env_prefers_active_over_fallback() {
        assert_eq!(
            select_agent_env(Some("haiku".into()), Some("developer".into())).as_deref(),
            Some("haiku")
        );
        assert_eq!(
            select_agent_env(None, Some("developer".into())).as_deref(),
            Some("developer")
        );
        assert_eq!(select_agent_env(Some(String::new()), None), None);
    }
}
