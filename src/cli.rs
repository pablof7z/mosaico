//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{
    dirty_label, format_local_datetime, now_secs, pubkey_short, relative_time, slugify_host,
};
use anyhow::{bail, Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use owo_colors::OwoColorize;
use shlex;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::time::{Duration, Instant};

mod admin;
mod args;
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
pub use args::Cli;
use args::{AgentAction, ChannelsAction, ChatAction, Cmd, DebugAction, ProjectAction, TmuxAction};
pub use messaging::{format_envelope, mention_short_id, EnvelopeView};
pub use turn::{assemble_turn_check_context, assemble_turn_start_context};
pub use who::load_who_snapshot;
pub(crate) use who::{new_agent_block, render_fabric_snapshot};

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

pub async fn run(cli: Cli) -> Result<()> {
    {
        let relays = crate::config::Config::load()
            .map(|c| c.relays.join(", "))
            .unwrap_or_else(|_| "none".to_string());
        let home = crate::config::edge_home();
        eprintln!("[tenex-edge] home={} relays={}", home.display(), relays);
    }
    // Any explicit command (except `hook`) signals intent to use tenex-edge, so
    // clear the stop-inhibit. Hooks honour the sentinel — they must never
    // restart a daemon the operator explicitly stopped. `stop` re-arms it
    // unconditionally, so clearing first is harmless.
    if !matches!(cli.cmd, Cmd::Hook { .. }) && crate::daemon::is_inhibited() {
        crate::daemon::clear_inhibit();
        eprintln!("[tenex-edge] stop inhibit cleared");
    }
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
            all_projects,
            live,
        } => {
            if live {
                who::who_live(project, all_projects)
            } else {
                who::who(project, all_projects)
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
        Cmd::Chat { action } => match action {
            ChatAction::Write {
                message,
                message_flag,
                channel,
            } => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::chat_write(message, channel).await
            }
            ChatAction::Read {
                since,
                limit,
                offset,
                tail,
                live,
                channel,
            } => messaging::chat_read(since, limit, offset, tail, live, channel).await,
        },
        Cmd::Statusline { session, tmux } => statusline::statusline(session, tmux),
        Cmd::Project { action } => admin::project(action).await,
        Cmd::Channels { action } => admin::channels(action).await,
        Cmd::Agent { action } => admin::agent(action).await,
        Cmd::Agents => admin::agents_roster().await,
        Cmd::Invite { agent } => admin::invite(agent).await,
        Cmd::Stop => stop_daemon(),
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
            DebugAction::Outbox {
                live,
                limit,
                refresh_ms,
            } => debug::outbox(live, limit, Duration::from_millis(refresh_ms.max(100))).await,
        },
        Cmd::Hook { host, hook_type } => hooks::hook_run(host, hook_type).await,
        Cmd::Tmux { action, popup } => match action {
            Some(action) => tmux_cli::tmux_run(action).await,
            None => tmux_cli::tmux_tui(popup),
        },
        Cmd::Launch {
            slug,
            project,
            channel,
            command_str,
            extra_args,
        } => {
            let override_command = command_str
                .map(|s| shlex::split(&s).unwrap_or_else(|| vec![s]))
                .unwrap_or_default();
            tmux_cli::launch(slug, project, channel, override_command, extra_args).await
        }
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

// Session resolution, session-id generation, recipient resolution, and the
// store live INSIDE the daemon now (it is the sole writer). The CLI verbs below
// are thin clients that forward to it over the UDS.

// ── stop ─────────────────────────────────────────────────────────────────────

fn stop_daemon() -> Result<()> {
    // Try to gracefully shut down a running daemon without spawning one.
    match crate::daemon::blocking::call_no_spawn("shutdown", serde_json::json!({})) {
        Ok(_) => eprintln!("[tenex-edge] daemon stopped"),
        Err(_) => eprintln!("[tenex-edge] daemon was not running"),
    }
    crate::daemon::set_inhibit();
    eprintln!(
        "[tenex-edge] hooks will not restart the daemon; \
         run any non-hook command (e.g. `tenex-edge who`) to resume"
    );
    Ok(())
}

// ── session-end ──────────────────────────────────────────────────────────────

pub(super) fn session_end(session: String) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    let v = crate::daemon::blocking::call("session_end", serde_json::json!({"session": session}))?;
    if v["ended"].as_bool().unwrap_or(false) {
        eprintln!("session {session} ended");
    } else {
        eprintln!("no such session: {session}");
    }
    Ok(())
}

/// Async daemon call for non-hook CLI verbs.
pub(super) async fn daemon_call_async(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

/// Hook-path daemon call: returns `Ok(Null)` when the daemon is inhibited
/// (after `tenex-edge stop`) so hooks fail open rather than spawning it.
pub(super) async fn daemon_call_hook_async(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    if crate::daemon::is_inhibited() {
        return Ok(serde_json::Value::Null);
    }
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

pub(super) async fn daemon_call_hook_async_with_items<F>(
    method: &str,
    params: serde_json::Value,
    on_item: F,
) -> Result<serde_json::Value>
where
    F: FnMut(serde_json::Value),
{
    if crate::daemon::is_inhibited() {
        return Ok(serde_json::Value::Null);
    }
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call_with_items(method, params, on_item).await
}

#[cfg(test)]
#[path = "cli/tests/turn_context.rs"]
mod turn_context_tests;

#[cfg(test)]
#[path = "cli/tests/tail_render.rs"]
mod tail_render_tests;
