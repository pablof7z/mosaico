//! The host-neutral CLI surface (M1 §6).

use crate::domain::DomainEvent;
use crate::state::Store;
use crate::util::{dirty_label, format_local_datetime, now_secs, pubkey_short, relative_time};
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
mod statusline;
mod tmux_cli;
mod turn;
mod who;

pub use admin::render_fabric;
#[cfg(test)]
use admin::{parse_since, render_tail_event};
pub use args::Cli;
use args::{
    AgentAction, ChannelsAction, ChatAction, Cmd, DebugAction, HarnessAction, ProjectAction,
};
pub use messaging::{format_envelope, mention_short_id, EnvelopeView};
pub use turn::{assemble_turn_check_context, assemble_turn_start_context};
pub(crate) use turn::{turn_check_audit, turn_start_audit};
pub use who::load_who_snapshot;
pub(crate) use who::{render_fabric_context, FabricContextInput};

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

/// The tmux pane (`$TMUX_PANE`) this CLI invocation runs in — the durable
/// in-session anchor. It is present in the harness env from process birth and is
/// 1:1 with the session, so the daemon resolves it (via the pane's `tmux_pane`
/// alias) to the caller's canonical session. Empty outside tmux (e.g. opencode),
/// where the daemon falls back to the agent+cwd scan. This REPLACES the old
/// `TENEX_EDGE_SESSION` env var, which could never be set (the canonical id is
/// minted only after the harness starts).
pub(crate) fn tmux_pane_env() -> Option<String> {
    std::env::var("TMUX_PANE").ok().filter(|s| !s.is_empty())
}

/// The caller-identity fields every in-session RPC sends so the daemon resolves
/// "which session am I" identically (SSOT — the daemon mirror is
/// `CallerAnchor::from_params`). One definition keeps senders from drifting (an
/// earlier hand-rolled `invite` payload silently dropped `tmux_pane`). Merge
/// call-specific fields on top with [`rpc_params`].
pub(crate) fn caller_identity() -> serde_json::Value {
    let tmux_pane = tmux_pane_env();
    let watch_anchor = if tmux_pane.is_none() {
        hooks::caller_watch_pid_anchor()
    } else {
        None
    };
    let (harness, watch_pid) = watch_anchor
        .map(|(harness, pid)| (Some(harness), Some(pid)))
        .unwrap_or((None, None));
    serde_json::json!({
        "tmux_pane": tmux_pane,
        "harness": harness,
        "watch_pid": watch_pid,
        "agent": agent_env_slug(),
        "cwd": std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        "group": channel_env(),
    })
}

/// Build RPC params = the caller-identity fields plus `extra` (which wins on any
/// key collision, e.g. an explicit destination `group`).
pub(crate) fn rpc_params(extra: serde_json::Value) -> serde_json::Value {
    let mut base = caller_identity();
    if let (Some(b), Some(e)) = (base.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            b.insert(k.clone(), v.clone());
        }
    }
    base
}

pub async fn run(cli: Cli) -> Result<()> {
    {
        let relays = crate::config::Config::load()
            .map(|c| c.relays.join(", "))
            .unwrap_or_else(|_| "none".to_string());
        let home = crate::config::edge_home();
        eprintln!("[tenex-edge] home={} relays={}", home.display(), relays);
    }
    // Any explicit command (except `harness hook`) signals intent to use tenex-edge, so
    // clear the stop-inhibit. Hooks honour the sentinel — they must never
    // restart a daemon the operator explicitly stopped. `stop` re-arms it
    // unconditionally, so clearing first is harmless.
    if !matches!(
        cli.cmd,
        Cmd::Harness {
            action: HarnessAction::Hook { .. }
        }
    ) && crate::daemon::is_inhibited()
    {
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
        Cmd::Chat { action } => match action {
            ChatAction::Write {
                message,
                message_flag,
                channel,
                long_message,
            } => {
                let message = messaging::resolve_send_message_body(message_flag.or(message))?;
                messaging::chat_write(message, channel, long_message).await
            }
            ChatAction::Read {
                id,
                since,
                limit,
                offset,
                tail,
                live,
                channel,
            } => messaging::chat_read(id, since, limit, offset, tail, live, channel).await,
        },
        Cmd::Project { action } => admin::project(action).await,
        Cmd::Doctor => admin::doctor().await,
        Cmd::Channels { action } => admin::channels(action).await,
        Cmd::Agent { action } => admin::agent(action).await,
        Cmd::Agents { action } => admin::agents(action).await,
        Cmd::Invite {
            channel,
            agent,
            session,
        } => admin::invite(channel, agent, session).await,
        Cmd::Harness { action } => match action {
            HarnessAction::Hook { harness, hook_type } => hooks::hook_run(harness, hook_type).await,
            HarnessAction::Statusline { session, tmux } => statusline::statusline(session, tmux),
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
        Cmd::Stop => stop_daemon(),
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
        Cmd::Daemon => crate::daemon::server::run().await,
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
