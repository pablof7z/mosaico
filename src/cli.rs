//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{
    dirty_label, format_local_datetime, now_secs, pubkey_short, relative_time, session_codename,
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
mod install;
mod messaging;
mod project_agents;
mod statusline;
mod tmux_cli;
mod turn;
mod who;

pub use admin::render_fabric;
#[cfg(test)]
use admin::{parse_since, render_tail_event};
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

/// The NIP-29 subgroup id (`h`) this pane was spawned into, exported as
/// `TENEX_EDGE_CHANNEL`. Present only for sessions launched into a subgroup task
/// room; absent for ordinary project sessions. Threaded into session-resolving
/// RPCs so the daemon binds to the subgroup session (stored under this `h`)
/// rather than a sibling parent-project session in the same working directory.
pub(crate) fn channel_env() -> Option<String> {
    std::env::var("TENEX_EDGE_CHANNEL")
        .ok()
        .filter(|s| !s.is_empty())
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
    /// Show your own identity on the fabric: agent slug, session codename,
    /// canonical session id, project, host, pubkey, and current status.
    Whoami {
        /// Session id; if omitted, resolved from env / the current directory.
        #[arg(long)]
        session: Option<String>,
        /// Emit the raw identity JSON instead of the rendered card.
        #[arg(long)]
        json: bool,
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
    /// Write or read NIP-29 project chat.
    Chat {
        #[command(subcommand)]
        action: ChatAction,
    },
    /// Manage NIP-29 project groups (list, set description).
    Project {
        #[command(subcommand)]
        action: ProjectAction,
    },
    /// Manage NIP-29 subgroup task channels under a project (create, list, switch).
    Channels {
        #[command(subcommand)]
        action: ChannelsAction,
    },
    /// Manage the local agent keystore: agents that have a private key on THIS
    /// machine under `<edge_home>/agents/<slug>.json`. These are the identities
    /// you can spawn locally; project membership is governed separately by the
    /// codec (e.g. the NIP-29 group's member list), not here.
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Render the one-line fabric statusline for a host's status bar.
    /// Reads the harness's statusline JSON payload on stdin (for `session_id`),
    /// prints one line, and always exits 0 — fails open when the daemon is down
    /// (and never spawns one).
    Statusline {
        /// Session id; if omitted, taken from the stdin payload or resolved from cwd.
        #[arg(long)]
        session: Option<String>,
        /// Agent slug to resolve when no session id is available (used by the
        /// tmux status-format invocation, which has no stdin payload).
        #[arg(long)]
        agent: Option<String>,
        /// Working directory override for project resolution (used by the tmux
        /// status-format invocation, which runs in the server's cwd, not the
        /// pane's project directory).
        #[arg(long)]
        cwd: Option<String>,
        /// Emit tmux #[style] format strings instead of ANSI codes. Required
        /// when the output is consumed by tmux's status-format (#(...)).
        #[arg(long)]
        tmux: bool,
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
    Publish {
        /// Proposal title.
        #[arg(long)]
        title: String,
        /// Proposal body (Markdown). Use "-" or omit to read from stdin.
        #[arg(long = "message", value_name = "BODY")]
        message: Option<String>,
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
        /// Extra args passed after `--`; appended to the launch command.
        /// Example: `tenex-edge launch codex -- --yolo`
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Detect local agent harnesses (Claude Code, Codex, opencode) and wire
    /// tenex-edge's hook entries into each. With no flags, opens a picker when
    /// interactive and selects detected harnesses in noninteractive shells.
    Install {
        /// Install into every detected harness (skip the interactive picker).
        #[arg(long)]
        all: bool,
        /// Comma-separated harness ids to install (e.g. `claude-code,codex`).
        /// Skips the picker.
        #[arg(long, value_name = "HARNESSES")]
        harness: Option<String>,
        /// Print exactly what would be written without changing anything.
        #[arg(long)]
        dry_run: bool,
        /// Show detection + install status for every known harness and exit.
        #[arg(long)]
        status: bool,
        /// Remove tenex-edge's hooks from the selected harnesses instead of
        /// installing.
        #[arg(long)]
        uninstall: bool,
    },
    /// Internal: the per-machine daemon. Spawned automatically; not for direct use.
    /// (Replaces the old detached per-session engine, which now runs as an async
    /// task inside this one daemon — the sole writer of state.db.)
    #[command(name = "__daemon", hide = true)]
    Daemon,
}

#[derive(Subcommand)]
enum ChatAction {
    /// Publish a project chat line. Reads body from arg, --message, or stdin.
    /// Mention a session inline by writing `@<codename>` in the body.
    Write {
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
        /// Session id (prefix, or codename like `bravo4217`) to resume.
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
enum AgentAction {
    /// List the agents in this machine's local keystore (slug, pubkey, command).
    List,
    /// Add a local agent: mint + persist its keypair if the slug is new. Pass a
    /// harness launch command after `--` to set how it spawns (e.g.
    /// `tenex-edge agent add reviewer -- claude --dangerously-skip-permissions`);
    /// re-running with a new command overwrites it. With no command, spawning
    /// falls back to the built-in defaults for claude/codex/opencode.
    ///
    /// Repeat `--project <p>` to also assign the agent to one or more projects
    /// in the same step (adds its pubkey to each NIP-29 group).
    Add {
        /// Agent slug ([A-Za-z0-9._-]).
        slug: String,
        /// Assign to this project (repeatable). Adds the agent's pubkey to the
        /// project's NIP-29 group.
        #[arg(long = "project", value_name = "PROJECT")]
        projects: Vec<String>,
        /// Harness launch command (everything after `--`). Optional.
        #[arg(last = true, value_name = "COMMAND")]
        command: Vec<String>,
    },
    /// Assign an existing local agent to one or more projects: add its pubkey to
    /// each project's NIP-29 group. Repeat `--project <p>` for multiple projects.
    /// Requires your operator key to be a group admin on the relay.
    Assign {
        /// Agent slug (must already exist in the local keystore).
        slug: String,
        /// Project to assign to (repeatable; at least one required).
        #[arg(long = "project", value_name = "PROJECT", required = true)]
        projects: Vec<String>,
    },
    /// Remove a local agent. Its key file is parked at `<slug>.json.removed`
    /// (not deleted) so a mistake is recoverable; the agent stops being spawnable
    /// and stops being auto-trusted on next read.
    Remove {
        /// Agent slug to remove.
        slug: String,
    },
}

#[derive(Subcommand)]
enum ProjectAction {
    /// List all NIP-29 project groups on the relay.
    List,
    /// Initialize the current directory as a tenex-edge project. Registers the
    /// directory's basename as a slug in `~/.tenex-edge/projects.json`. Refuses
    /// if the slug is already mapped to a different path; pass `--force` to
    /// overwrite. No-op if the slug is already mapped to this exact path.
    Init {
        /// Overwrite an existing slug→path mapping that points elsewhere.
        #[arg(long)]
        force: bool,
    },
    /// Set the description for a project's NIP-29 group (publishes kind:9002).
    Edit {
        /// New description text.
        #[arg(long)]
        description: String,
        /// Project slug; defaults to the project resolved from the current directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Edit the current project's local-agent membership, or add one local agent/pubkey.
    Add {
        /// Project slug. Omit to use the project resolved from the current directory.
        project: Option<String>,
        /// Local agent slug, hex pubkey, npub, or NIP-05 address. When omitted,
        /// opens a picker of local agents and publishes the needed put-user/remove-user events.
        #[arg(value_name = "AGENT_OR_PUBKEY")]
        pubkey: Option<String>,
    },
}

/// Subgroup task channels under a project (NIP-29 child groups).
#[derive(Subcommand)]
enum ChannelsAction {
    /// Create a subgroup task channel under a project and publish one kind:9
    /// orchestration event asking the named backends to add their agents. The
    /// agent that runs this command is auto-added to the new channel.
    Create {
        /// Human-readable channel name, e.g. "support". The child group id
        /// (NIP-29 `h` value) becomes "<slugified-name>-<random8>".
        #[arg(long)]
        name: String,
        /// Repeatable `slug@backend`, where `slug` is the agent identity (the
        /// `~/.tenex-edge/agents/*.json` filename stem, e.g. `developer`, `alice`)
        /// and `backend` is a hex pubkey or npub of the target backend (the pubkey
        /// of its tenexPrivateKey).
        #[arg(long = "agent", value_name = "SLUG@BACKEND")]
        agents: Vec<String>,
        /// Parent project slug this channel hangs under. Defaults to the project
        /// resolved from the current directory.
        #[arg(long)]
        project: Option<String>,
        /// Path to a markdown brief; its contents become the kind:9 prose body.
        #[arg(long = "message", value_name = "PATH")]
        message: Option<PathBuf>,
    },
    /// List the subgroup task channels under a project.
    List {
        /// Parent project slug. Defaults to the project resolved from the current
        /// directory.
        #[arg(long)]
        project: Option<String>,
    },
    /// Switch the active channel for the current tmux pane to a different NIP-29 subgroup.
    Switch {
        /// The NIP-29 `h` value of the subgroup to switch to.
        channel: String,
    },
}

#[derive(Subcommand)]
enum DebugAction {
    /// Live TUI for hook injections and tenex-edge command invocations.
    HookTail {
        /// Filter panes/events to one or more projects (repeatable).
        #[arg(long = "project")]
        projects: Vec<String>,
        /// Filter panes/events to a session id or codename.
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
        Cmd::Publish {
            title,
            message,
            d,
            session,
        } => {
            let body = messaging::resolve_send_message_body(message)?;
            messaging::publish(title, body, d, session).await
        }
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
        Cmd::Whoami { session, json } => who::whoami(session, json).await,
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
        Cmd::Chat { action } => match action {
            ChatAction::Write {
                message,
                message_flag,
                session,
            } => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::chat_write(message, session).await
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
        Cmd::Statusline {
            session,
            agent,
            cwd,
            tmux,
        } => statusline::statusline(session, agent, cwd, tmux),
        Cmd::Project { action } => admin::project(action).await,
        Cmd::Channels { action } => admin::channels(action).await,
        Cmd::Agent { action } => admin::agent(action).await,
        Cmd::Doctor => admin::doctor().await,
        Cmd::Debug { action } => match action {
            DebugAction::HookTail {
                projects,
                session,
                panes,
                refresh_ms,
            } => debug::hook_tail(debug::HookTailOpts {
                projects,
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
        Cmd::Launch {
            slug,
            project,
            command,
        } => tmux_cli::launch(slug, project, command).await,
        Cmd::Daemon => crate::daemon::server::run().await,
        Cmd::Install {
            all,
            harness,
            dry_run,
            status,
            uninstall,
        } => {
            install::install(install::InstallOpts {
                all,
                harness,
                dry_run,
                status,
                uninstall,
            })
            .await
        }
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

pub(super) async fn daemon_call_async_with_items<F>(
    method: &str,
    params: serde_json::Value,
    on_item: F,
) -> Result<serde_json::Value>
where
    F: FnMut(serde_json::Value),
{
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call_with_items(method, params, on_item).await
}

// ── freeze tests — turn-start / turn-check context assembly ─────────────────

#[cfg(test)]
mod turn_context_tests {
    use super::*;
    use crate::session::{Harness, SessionObservation};
    use crate::state::{SessionRecord, Store};
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

    /// Build a minimal alive SessionRecord for testing context assembly.
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

    /// turn_start returns None on a non-first turn with no chat and no peers.
    #[test]
    fn turn_start_context_returns_none_when_empty_non_first_turn() {
        let store = Store::open_memory().unwrap();
        let rec = test_session("sess-freeze-2");
        // No chat rows. Non-first turn (prev != 0). No peer sessions.
        let m = Mutex::new(store);

        let ctx = assemble_turn_start_context(&m, &rec, /* prev_turn_started_at */ 42);
        assert!(
            ctx.is_none(),
            "turn_start with no chat, non-first turn, no peers must return None; got: {ctx:?}"
        );
    }

    /// turn_check returns None when there is no chat and delta_since=None (rate-limited).
    #[test]
    fn turn_check_context_returns_none_when_nothing_due() {
        let store = Store::open_memory().unwrap();
        let m = Mutex::new(store);
        let ctx =
            assemble_turn_check_context(&m, &test_session("sess-no-rows"), "laptop", None, 200);
        assert!(
            ctx.is_none(),
            "turn_check with no chat, no delta → None; got: {ctx:?}"
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
        // Changed renders as a canonical presence line: `* codename (agent@host) — activity`.
        assert!(
            text.contains("(sib@laptop) — editing hooks.rs"),
            "sibling activity expected as a canonical presence line; got: {text:?}"
        );
        assert!(
            !text.contains("My own work"),
            "viewer's own status must be excluded; got: {text:?}"
        );
        // The session must render as the targetable codename (matching `who`),
        // never the raw id — otherwise it can't be copied into `send --to-session`.
        assert!(
            text.contains(&crate::util::session_codename(&sib_id)),
            "session must render as codename; got: {text:?}"
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
            text.contains("(sib@laptop) — idle"),
            "idle marker expected in the canonical presence line; got: {text:?}"
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

    fn chat_row(session_id: &str, eid: &str, created_at: u64) -> crate::state::ChatInboxRow {
        crate::state::ChatInboxRow {
            chat_event_id: eid.to_string(),
            target_session: session_id.to_string(),
            from_pubkey: "pk-chat".to_string(),
            from_slug: "chatter".to_string(),
            project: "proj".to_string(),
            body: "ambient chatter".to_string(),
            created_at,
            from_session: String::new(),
            mentioned_session: String::new(),
        }
    }

    /// Project chat is delta-gated: a row newer than the cursor surfaces once,
    /// but a row older than the cursor (already shown earlier this turn) does
    /// not re-emit on the next tool call. The peek never marks it delivered, so
    /// without the cursor filter it would repeat on every PostToolUse.
    #[test]
    fn turn_check_chat_shown_once_not_per_tool_call() {
        let store = Store::open_memory().unwrap();
        // Arrived at 120, after the cursor (50) → surfaces on this check.
        store
            .enqueue_chat(&chat_row("sess-me", "chat-new", 120))
            .unwrap();
        let m = Mutex::new(store);

        let text =
            assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(50), 200)
                .expect("fresh chat past the cursor must surface");
        assert!(
            text.contains("[tenex-edge] Project chat while you were working:"),
            "chat block expected; got: {text:?}"
        );

        // Next check's cursor has advanced past the row (since=150 > 120): the
        // same undelivered row must NOT re-emit.
        let text2 =
            assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", Some(150), 200);
        assert!(
            text2.is_none(),
            "already-shown chat must not repeat once the cursor passes it; got: {text2:?}"
        );

        // The row is still undelivered (peek, not drain) so turn_start delivers it.
        let g = m.lock().unwrap();
        assert_eq!(g.peek_chat("sess-me").unwrap().len(), 1);
    }

    /// `delta_since = None` (rate-limited / not mid-turn) suppresses project chat
    /// too, not just the sibling delta — chat is debounced, the inbox is not.
    #[test]
    fn turn_check_chat_suppressed_when_not_due() {
        let store = Store::open_memory().unwrap();
        store
            .enqueue_chat(&chat_row("sess-me", "chat-x", 120))
            .unwrap();
        let m = Mutex::new(store);

        let ctx = assemble_turn_check_context(&m, &test_session("sess-me"), "laptop", None, 200);
        assert!(
            ctx.is_none(),
            "chat must be suppressed when not due (no inbox to surface); got: {ctx:?}"
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
        // Canonical sender identity: `codename (agent@host)`.
        assert_eq!(
            lines[0],
            format!(
                "From: {} (codex@my-box)",
                session_codename("sender-session-id")
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
        // Host (slugified) rides in the canonical `agent@host`; no `[remote:]` tag.
        assert!(out.contains("(codex@prod-01-example-com)"));
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
            body: "can you review the codec?".into(),
        };
        let line = render_tail_event(&ev, false, false, false, false);
        assert!(line.starts_with(&ts_str()), "should start with timestamp");
        assert!(line.contains("msg"), "should contain category");
        assert!(line.contains("claude@proj"), "should contain agent@project");
        assert!(line.contains("->"), "ASCII arrow when no_emoji");
        assert!(line.contains("codex"), "should contain recipient");
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
