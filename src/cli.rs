use crate::idref::event_short_id;
#[cfg(test)]
use crate::state::Store;
use crate::util::{format_local_datetime, now_secs, pubkey_short};
use anyhow::{bail, Context, Result};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{self, Event as TermEvent, KeyCode, KeyModifiers},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use owo_colors::OwoColorize;
use std::fmt::Write as _;
use std::io::{self, IsTerminal as _, Read as _, Write as _};
use std::time::Duration;

mod acp_smoke;
mod admin;
mod agents;
mod args;
mod context;
mod daemon_lifecycle;
mod debug;
mod dispatch;
mod doctor;
mod explain;
mod harness;
mod hooks;
pub mod install;
mod interactive;
mod launch_cli;
mod mcp;
mod messaging;
mod my;
mod pty;
mod relay;
mod resume;
mod session;
mod statusline;
mod turn;
mod who;

#[cfg(test)]
use admin::{parse_since, render_tail_event};
pub use args::{print_help_all, print_help_contextual, Cli};
use args::{Cmd, DaemonAction};
pub(crate) fn select_agent_env(active: Option<String>) -> Option<String> {
    active.filter(|s| !s.is_empty())
}

pub(crate) fn agent_env_slug() -> Option<String> {
    select_agent_env(std::env::var("MOSAICO_AGENT").ok())
}

/// The NIP-29 subgroup id (`h`) this PTY session was spawned into, exported as
/// `MOSAICO_CHANNEL`. Present only for sessions launched into a subgroup task
/// room; absent for ordinary channel sessions. Threaded into session-resolving
/// RPCs so the daemon binds to the subgroup session (stored under this `h`)
/// rather than a sibling parent-channel session in the same working directory.
pub(crate) fn channel_env() -> Option<String> {
    std::env::var("MOSAICO_CHANNEL")
        .ok()
        .filter(|s| !s.is_empty())
}

/// The hosted PTY session this CLI invocation runs in. It is present in the
/// harness env from process birth and is 1:1 with the session, so the daemon
/// resolves its typed PTY locator to the caller's pubkey. Native harness shells
/// fall back to native locators, watched pid, or channel scan.
pub(crate) fn pty_session_env() -> Option<String> {
    std::env::var("MOSAICO_PTY_SESSION")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Build the typed caller identity and channel it into the daemon's stable RPC
/// JSON shape. One definition keeps senders from drifting; merge call-specific
/// fields on top with [`rpc_params`].
pub(crate) fn caller_identity() -> serde_json::Value {
    context::InvocationContext::from_current_process().to_rpc_json()
}

/// Build RPC params from caller identity plus explicit non-null overrides.
pub(crate) fn rpc_params(extra: serde_json::Value) -> serde_json::Value {
    context::merge_rpc_params(caller_identity(), extra)
}

pub async fn run(cli: Cli) -> Result<()> {
    // Any explicit command (except `harness hook`) signals intent to use mosaico, so
    // clear the daemon stop-inhibit. Hooks honour the sentinel — they must never
    // restart a daemon the operator explicitly stopped. `daemon stop` re-arms it
    // unconditionally, so clearing first is harmless.
    if !matches!(
        cli.cmd.as_ref(),
        Some(Cmd::Harness { action }) if action.is_hook()
    ) && crate::daemon::is_inhibited()
    {
        crate::daemon::clear_inhibit();
        eprintln!("[mosaico] stop inhibit cleared");
    }
    match cli.cmd {
        None => interactive::session_picker::home().await,
        Some(Cmd::Who(args)) => who::who(args),
        Some(Cmd::Resume(args)) => resume::resume(args).await,
        Some(Cmd::Channel { action }) => admin::channels(action).await,
        Some(Cmd::Wait(args)) => messaging::wait(args).await,
        Some(Cmd::Agents(args)) => agents::agents(args).await,
        Some(Cmd::Dispatch(args)) => dispatch::dispatch(args).await,
        Some(Cmd::Harness { action }) => harness::harness(action).await,
        Some(Cmd::Mcp(args)) => mcp::mcp(args).await,
        Some(Cmd::My { action }) => my::my(action),
        Some(Cmd::Daemon(args)) => match args.action {
            Some(DaemonAction::Restart) => daemon_lifecycle::restart().await,
            Some(DaemonAction::Stop) => daemon_lifecycle::stop(),
            None => crate::daemon::server::run().await,
        },
        Some(Cmd::Relay(args)) => relay::relay(args),
        Some(Cmd::Debug { action }) => debug::debug(action).await,
        Some(Cmd::Doctor(args)) => doctor::doctor(args).await,
        Some(Cmd::PtySupervisor(args)) => pty::pty_supervisor(args),
        Some(Cmd::Install(args)) => install::install(args).await,
        Some(Cmd::AcpSmoke(args)) => acp_smoke::acp_smoke(args).await,
        Some(Cmd::Fallback(args)) => {
            launch_cli::verbs::launch(launch_cli::LaunchRequest::from_external(args)?).await
        }
    }
}

// Session resolution and storage live in the daemon; CLI verbs are thin UDS clients.

// ── session-end ──────────────────────────────────────────────────────────────

pub(super) fn session_end(session: String) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    let v = crate::daemon::blocking::call(
        "session_end",
        serde_json::json!({"session": session, "cause": "manual"}),
    )?;
    if v["ended"].as_bool().unwrap_or(false) {
        eprintln!("session {session} ended");
    } else {
        eprintln!("no such session: {session}");
    }
    Ok(())
}

/// Ask the daemon's `session_kill` RPC to stop this session's hosted process
/// and mark it offline (the process-termination counterpart to
/// [`session_end`], which only touches metadata). Exits non-zero when the
/// daemon reports process termination failed.
pub(super) fn session_kill(session: String) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    let v = crate::daemon::blocking::call("session_kill", serde_json::json!({"session": session}))?;
    if v["killed"].as_bool().unwrap_or(false) {
        eprintln!("session {session} killed");
        return Ok(());
    }
    let reason = v["reason"].as_str().unwrap_or("process termination failed");
    bail!("could not kill session {session}: {reason}")
}

/// Re-home the caller's own session into a fresh daemon-owned PTY. Runs
/// server-side as one RPC (`session_pty_wrap`) — see
/// `src/daemon/server/session_pty_wrap.rs` for the kill-then-resume sequence.
pub(super) fn session_pty_wrap_me(session: String) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    let v = crate::daemon::blocking::call(
        "session_pty_wrap",
        serde_json::json!({
            "session": session,
            "interrupt_working": false,
            "turn_count": 0,
        }),
    )?;
    if v["wrapped"].as_bool().unwrap_or(false) {
        let pty_id = v["pty_id"].as_str().unwrap_or("?");
        eprintln!("session {session} re-homed into daemon PTY {pty_id}");
        return Ok(());
    }
    let reason = v["reason"].as_str().unwrap_or("re-home refused");
    if v["refusal"].as_str() == Some("already_wrapped") {
        eprintln!("{reason}");
        return Ok(());
    }
    bail!("cannot pty-wrap-me: {reason}")
}

pub(super) fn session_end_hook(session: String, harness: &str) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    let pubkey = std::env::var("MOSAICO_PUBKEY")
        .ok()
        .filter(|value| !value.is_empty());
    if let Err(e) = crate::daemon::blocking::call_no_spawn(
        "session_end",
        serde_json::json!({
            "session": pubkey,
            "harness_session": session,
            "harness": harness,
            "cause": "harness_hook",
        }),
    ) {
        eprintln!("[mosaico] session-end hook skipped: {e:#}");
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

/// Hard caps for hook daemon calls. Hooks are on the harness critical path and
/// must fail open quickly; slow relay proof happens outside the hook response.
const HOOK_DAEMON_TIMEOUT: Duration = Duration::from_secs(5);

/// Hook-path daemon call: returns `Ok(Null)` when the daemon is inhibited
/// (after `mosaico daemon stop`) so hooks fail open rather than spawning it.
pub(super) async fn daemon_call_hook_async(
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    if crate::daemon::is_inhibited() {
        return Ok(serde_json::Value::Null);
    }
    tokio::time::timeout(HOOK_DAEMON_TIMEOUT, async {
        let mut client = crate::daemon::client::Client::connect_running().await?;
        client.call(method, params).await
    })
    .await
    .context("hook: timed out talking to daemon")?
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
    tokio::time::timeout(HOOK_DAEMON_TIMEOUT, async {
        let mut client = crate::daemon::client::Client::connect_running().await?;
        client.call_with_items(method, params, on_item).await
    })
    .await
    .context("hook: timed out talking to daemon")?
}

/// Runs a blocking hook-path daemon call (the sync `daemon::blocking` client
/// used by `turn_check`/`turn_end`) on a blocking thread, under the same
/// [`HOOK_DAEMON_TIMEOUT`] as the async hook path — hooks must never hang
/// regardless of which client a given RPC happens to use.
pub(super) async fn run_hook_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::time::timeout(HOOK_DAEMON_TIMEOUT, tokio::task::spawn_blocking(f))
        .await
        .context("hook: timed out talking to daemon")?
        .context("hook: blocking daemon call panicked")?
}

#[cfg(test)]
#[path = "cli/tests/turn_context.rs"]
mod turn_context_tests;

#[cfg(test)]
#[path = "cli/tests/tail_render.rs"]
mod tail_render_tests;
