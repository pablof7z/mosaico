//! The host-neutral CLI surface (M1 §6).

use crate::idref::event_short_id;
#[cfg(test)]
use crate::state::Store;
use crate::util::{format_local_datetime, now_secs, pubkey_short, relative_time};
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
use std::time::{Duration, Instant};

mod acp_smoke;
mod admin;
mod args;
mod config;
mod context;
mod debug;
mod dispatch;
mod explain;
mod harness;
mod hooks;
mod install;
mod interactive;
mod launch_cli;
mod mcp;
mod messaging;
mod my;
mod probe;
mod pty;
mod session;
mod statusline;
mod turn;
mod validate;
mod who;

#[cfg(test)]
use admin::{parse_since, render_tail_event};
pub use args::Cli;
use args::{Cmd, DaemonAction, MgmtAction, MgmtSessionAction};
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

/// The NIP-29 subgroup id (`h`) this PTY session was spawned into, exported as
/// `TENEX_EDGE_CHANNEL`. Present only for sessions launched into a subgroup task
/// room; absent for ordinary channel sessions. Threaded into session-resolving
/// RPCs so the daemon binds to the subgroup session (stored under this `h`)
/// rather than a sibling parent-channel session in the same working directory.
pub(crate) fn channel_env() -> Option<String> {
    std::env::var("TENEX_EDGE_CHANNEL")
        .ok()
        .filter(|s| !s.is_empty())
}

pub(crate) fn ephemeral_session_env() -> bool {
    std::env::var("TENEX_EDGE_EPHEMERAL").ok().as_deref() == Some("1")
}

/// The hosted PTY session this CLI invocation runs in. It is present in the
/// harness env from process birth and is 1:1 with the session, so the daemon
/// resolves it to the caller's canonical session. Native harness shells outside
/// tenex-edge launch fall back to harness ids, watched pid, or channel scan.
pub(crate) fn pty_session_env() -> Option<String> {
    std::env::var("TENEX_EDGE_PTY_SESSION")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Build the typed caller identity and channel it into the daemon's stable RPC
/// JSON shape. One definition keeps senders from drifting; merge call-specific
/// fields on top with [`rpc_params`].
pub(crate) fn caller_identity() -> serde_json::Value {
    context::InvocationContext::from_current_process().to_rpc_json()
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
    // Any explicit command (except `harness hook`) signals intent to use tenex-edge, so
    // clear the daemon stop-inhibit. Hooks honour the sentinel — they must never
    // restart a daemon the operator explicitly stopped. `daemon stop` re-arms it
    // unconditionally, so clearing first is harmless.
    if !matches!(
        cli.cmd,
        Cmd::Harness { ref action } if action.is_hook()
    ) && crate::daemon::is_inhibited()
    {
        crate::daemon::clear_inhibit();
        eprintln!("[tenex-edge] stop inhibit cleared");
    }
    match cli.cmd {
        Cmd::Publish(args) => messaging::publish(args).await,
        Cmd::Who(args) => who::who(args),
        Cmd::Channel { action } => admin::channels(action).await,
        Cmd::Wait(args) => messaging::wait(args).await,
        Cmd::Agents { action } => admin::agents(action).await,
        Cmd::Mgmt { action } => match action {
            MgmtAction::Agent { action } => admin::agent(action).await,
            MgmtAction::Session {
                action: MgmtSessionAction::List,
            } => interactive::session_picker::session_list().await,
            MgmtAction::Config(args) => config::config(args).await,
        },
        Cmd::Dispatch(args) => dispatch::dispatch(args).await,
        Cmd::Harness { action } => harness::harness(action).await,
        Cmd::Launch(args) => launch_cli::launch(args).await,
        Cmd::Mcp(args) => mcp::mcp(args).await,
        Cmd::My { action } => my::my(action),
        Cmd::Daemon(args) => match args.action {
            Some(DaemonAction::Restart) => restart_daemon().await,
            Some(DaemonAction::Stop) => stop_daemon(),
            None => crate::daemon::server::run().await,
        },
        Cmd::Debug { action } => debug::debug(action).await,
        Cmd::Probe(args) => probe::probe(args).await,
        Cmd::Pty { action } => pty::pty(action),
        Cmd::PtySupervisor(args) => pty::pty_supervisor(args),
        Cmd::Install(args) => install::install(args).await,
        Cmd::AcpSmoke(args) => acp_smoke::acp_smoke(args).await,
    }
}

// Session resolution, session-id generation, recipient resolution, and the
// store live INSIDE the daemon now (it is the sole writer). The CLI verbs below
// are thin clients that forward to it over the UDS.

// ── daemon lifecycle ─────────────────────────────────────────────────────────

/// How long `stop` waits for a shut-down daemon to actually exit (release its
/// startup flock) before giving up and reporting it as still-running.
const DAEMON_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

fn stop_daemon() -> Result<()> {
    request_daemon_shutdown();
    crate::daemon::set_inhibit();
    eprintln!(
        "[tenex-edge] hooks will not restart the daemon; \
         run `tenex-edge daemon restart` to resume"
    );
    Ok(())
}

async fn restart_daemon() -> Result<()> {
    if !request_daemon_shutdown() {
        bail!("daemon shutdown did not complete; refusing to start a second daemon")
    }

    crate::daemon::clear_inhibit();
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call("ping", serde_json::json!({})).await?;
    eprintln!("[tenex-edge] daemon restarted");
    Ok(())
}

/// Ask a running daemon to exit without spawning one. Returns whether it is
/// safe for a caller to start a replacement daemon.
fn request_daemon_shutdown() -> bool {
    match crate::daemon::blocking::call_no_spawn("shutdown", serde_json::json!({})) {
        Ok(_) => wait_for_daemon_exit(),
        Err(_) => {
            eprintln!("[tenex-edge] daemon was not running");
            true
        }
    }
}

/// The RPC layer acks `shutdown` the instant it wakes the daemon's shutdown
/// future — before the daemon has actually torn down the relay connection,
/// removed its socket, and dropped its startup flock. Poll that flock
/// (non-blocking `try_acquire`) so `stop` doesn't return until the old
/// process has genuinely exited and released it.
fn wait_for_daemon_exit() -> bool {
    let deadline = Instant::now() + DAEMON_SHUTDOWN_TIMEOUT;
    loop {
        match crate::daemon::client::StartupLock::try_acquire() {
            Ok(Some(_lock)) => {
                eprintln!("[tenex-edge] daemon stopped");
                return true;
            }
            Ok(None) => {}
            Err(e) => {
                eprintln!("[tenex-edge] daemon shutdown requested but could not confirm exit: {e}");
                return false;
            }
        }
        if Instant::now() >= deadline {
            eprintln!(
                "[tenex-edge] daemon shutdown requested but it did not exit within {DAEMON_SHUTDOWN_TIMEOUT:?}"
            );
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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
    let v =
        crate::daemon::blocking::call("session_pty_wrap", serde_json::json!({"session": session}))?;
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

pub(super) fn session_end_hook(session: String) -> Result<()> {
    if crate::daemon::is_inhibited() {
        return Ok(());
    }
    if let Err(e) = crate::daemon::blocking::call_no_spawn(
        "session_end",
        serde_json::json!({"session": session}),
    ) {
        eprintln!("[tenex-edge] session-end hook skipped: {e:#}");
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
/// (after `tenex-edge daemon stop`) so hooks fail open rather than spawning it.
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
