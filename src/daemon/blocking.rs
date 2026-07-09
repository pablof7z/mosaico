//! A synchronous (blocking) thin client for CLI verbs that are NOT `async`.
//!
//! The async [`super::client::Client`] can't be `block_on`'d from inside the
//! tokio runtime the CLI already runs on (that panics). Sync verbs instead use
//! this plain `std::os::unix::net::UnixStream` client: spawn-if-absent via the
//! same `connect_or_spawn` mechanics is async, so for the blocking path we do a
//! minimal connect-or-spawn against the socket directly. Blocking I/O on a
//! one-shot CLI invocation is fine.

use super::protocol::{
    client_hello, daemon_too_new_message, handshake_decision, please_exit, HandshakeDecision,
    DAEMON_HANDSHAKE_IO_TIMEOUT, DAEMON_RESPAWN_GRACE, DAEMON_STARTUP_TIMEOUT,
};
use super::socket_path;
use super::spawn::spawn_detached_daemon;
use crate::config;
use anyhow::{anyhow, bail, Context, Result};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const DEFAULT_RESPONSE_IO_TIMEOUT: Duration = Duration::from_secs(2);
const SLOW_RESPONSE_IO_TIMEOUT: Duration = Duration::from_secs(25);

/// Connect to the daemon (spawning if absent), do the handshake, send one
/// request, and return the `ok` payload. Mirrors the async client's behavior
/// including the version-skew exit+respawn, but synchronously.
pub fn call(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    call_with_attempt(method, &params, try_call, spawn_if_absent)
}

fn call_with_attempt<F, S>(
    method: &str,
    params: &serde_json::Value,
    mut attempt: F,
    mut spawn: S,
) -> Result<serde_json::Value>
where
    F: FnMut(&str, &serde_json::Value) -> std::result::Result<Outcome, TryCallFailure>,
    S: FnMut() -> Result<()>,
{
    for _ in 0..5 {
        match attempt(method, params) {
            Ok(Outcome::Ok(v)) => return Ok(v),
            Ok(Outcome::Err(_code, msg)) => bail!("{msg}"),
            Ok(Outcome::SkewExit) => {
                std::thread::sleep(DAEMON_RESPAWN_GRACE);
                spawn()?;
            }
            Err(e) => {
                if e.to_string().contains("is newer than this binary") {
                    return Err(e.into_error());
                }
                if e.phase == FailurePhase::RequestMayHaveBeenDelivered
                    && !method_policy(method).retry_after_delivery
                {
                    bail!(
                        "daemon call {method} may have been processed, but no response was \
                         received ({e}). Not retrying automatically because the method is not \
                         declared idempotent."
                    );
                }
                spawn()?;
            }
        }
    }
    bail!("could not complete daemon call {method}")
}

/// One-shot call that NEVER spawns the daemon. For high-frequency fail-open
/// surfaces (the statusline) that must render nothing when no daemon is running
/// rather than booting one just to draw a line.
pub fn call_no_spawn(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    call_no_spawn_with_attempt(method, &params, try_call)
}

fn call_no_spawn_with_attempt<F>(
    method: &str,
    params: &serde_json::Value,
    mut attempt: F,
) -> Result<serde_json::Value>
where
    F: FnMut(&str, &serde_json::Value) -> std::result::Result<Outcome, TryCallFailure>,
{
    match attempt(method, params).map_err(TryCallFailure::into_error)? {
        Outcome::Ok(v) => Ok(v),
        Outcome::Err(_code, msg) => bail!("{msg}"),
        Outcome::SkewExit => bail!("daemon protocol skew; awaiting respawn"),
    }
}

enum Outcome {
    Ok(serde_json::Value),
    Err(String, String),
    SkewExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailurePhase {
    BeforeRequest,
    RequestMayHaveBeenDelivered,
}

#[derive(Debug)]
struct TryCallFailure {
    phase: FailurePhase,
    error: anyhow::Error,
}

impl TryCallFailure {
    fn before(error: anyhow::Error) -> Self {
        Self {
            phase: FailurePhase::BeforeRequest,
            error,
        }
    }

    fn after_request(error: anyhow::Error) -> Self {
        Self {
            phase: FailurePhase::RequestMayHaveBeenDelivered,
            error,
        }
    }

    fn into_error(self) -> anyhow::Error {
        self.error
    }
}

impl fmt::Display for TryCallFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for TryCallFailure {}

fn try_call(
    method: &str,
    params: &serde_json::Value,
) -> std::result::Result<Outcome, TryCallFailure> {
    let stream = UnixStream::connect(socket_path())
        .context("connecting to daemon socket")
        .map_err(TryCallFailure::before)?;
    stream
        .set_read_timeout(Some(DAEMON_HANDSHAKE_IO_TIMEOUT))
        .map_err(|e| TryCallFailure::before(e.into()))?;
    stream
        .set_write_timeout(Some(DAEMON_HANDSHAKE_IO_TIMEOUT))
        .map_err(|e| TryCallFailure::before(e.into()))?;
    let mut w = stream
        .try_clone()
        .map_err(|e| TryCallFailure::before(e.into()))?;
    let mut r = BufReader::new(stream);

    // hello → welcome.
    let hello =
        serde_json::to_string(&client_hello()).map_err(|e| TryCallFailure::before(e.into()))?;
    writeln!(w, "{hello}").map_err(|e| TryCallFailure::before(e.into()))?;
    let mut welcome_line = String::new();
    if r.read_line(&mut welcome_line)
        .map_err(|e| TryCallFailure::before(e.into()))?
        == 0
    {
        return Err(TryCallFailure::before(anyhow!(
            "daemon closed before welcome"
        )));
    }
    let welcome: serde_json::Value =
        serde_json::from_str(welcome_line.trim()).map_err(|e| TryCallFailure::before(e.into()))?;
    let dproto = welcome["protocol"].as_u64().unwrap_or(0) as u32;
    match handshake_decision(dproto) {
        HandshakeDecision::Ready => {}
        HandshakeDecision::AskOlderDaemonToExit => {
            // Older daemon under a newer binary: ask it to exit, then respawn.
            let exit_frame = serde_json::to_string(&please_exit())
                .map_err(|e| TryCallFailure::before(e.into()))?;
            writeln!(w, "{exit_frame}").map_err(|e| TryCallFailure::before(e.into()))?;
            let _ = w.flush();
            return Ok(Outcome::SkewExit);
        }
        HandshakeDecision::DaemonTooNew {
            daemon_protocol,
            client_protocol,
        } => {
            return Err(TryCallFailure::before(anyhow!(daemon_too_new_message(
                daemon_protocol,
                client_protocol
            ))));
        }
    }

    // request → response.
    let request = serde_json::json!({"id": 1, "method": method, "params": params}).to_string();
    writeln!(w, "{request}").map_err(|e| TryCallFailure::after_request(e.into()))?;
    r.get_ref()
        .set_read_timeout(Some(method_policy(method).response_timeout))
        .map_err(|e| TryCallFailure::after_request(e.into()))?;
    let mut resp_line = String::new();
    if r.read_line(&mut resp_line)
        .map_err(|e| TryCallFailure::after_request(e.into()))?
        == 0
    {
        return Err(TryCallFailure::after_request(anyhow!(
            "daemon closed the connection"
        )));
    }
    let resp: serde_json::Value = serde_json::from_str(resp_line.trim())
        .map_err(|e| TryCallFailure::after_request(e.into()))?;
    if let Some(err) = resp.get("error") {
        let code = err["code"].as_str().unwrap_or("error").to_string();
        let msg = err["message"].as_str().unwrap_or("").to_string();
        return Ok(Outcome::Err(code, msg));
    }
    Ok(Outcome::Ok(
        resp.get("ok").cloned().unwrap_or(serde_json::Value::Null),
    ))
}

#[derive(Clone, Copy)]
struct MethodPolicy {
    response_timeout: Duration,
    retry_after_delivery: bool,
}

fn method_policy(method: &str) -> MethodPolicy {
    match method {
        "ping" | "who" | "pty_status" | "pty_resumable" | "channel_members" => MethodPolicy {
            response_timeout: DEFAULT_RESPONSE_IO_TIMEOUT,
            retry_after_delivery: true,
        },
        "pty_spawn" => MethodPolicy {
            response_timeout: SLOW_RESPONSE_IO_TIMEOUT,
            retry_after_delivery: false,
        },
        _ => MethodPolicy {
            response_timeout: DEFAULT_RESPONSE_IO_TIMEOUT,
            retry_after_delivery: false,
        },
    }
}

/// Synchronous spawn-if-absent: under the startup `flock`, reclaim a stale
/// socket and spawn a detached daemon, then poll-connect.
fn spawn_if_absent() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;

    let mut noted_wait = false;
    let mut spawned_child: Option<std::process::Child> = None;
    let wait_deadline = Instant::now() + DAEMON_STARTUP_TIMEOUT;
    while Instant::now() < wait_deadline {
        if daemon_answers_ping() {
            return Ok(());
        }
        if let Some(lock) = super::client::StartupLock::try_acquire()? {
            eprintln!("[tenex-edge] starting daemon...");
            let sock = socket_path();
            if sock.exists() {
                let _ = std::fs::remove_file(&sock);
            }
            spawned_child = Some(spawn_detached_daemon()?);
            drop(lock);
            break;
        }
        if !noted_wait {
            eprintln!("[tenex-edge] waiting for daemon to finish startup...");
            noted_wait = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let mut noted_ready = false;
    let deadline = Instant::now() + DAEMON_STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if daemon_answers_ping() {
            return Ok(());
        }
        if let Some(child) = spawned_child.as_mut() {
            match super::poll_spawned_child(child) {
                super::SpawnedChildStatus::LostRace => spawned_child = None,
                super::SpawnedChildStatus::Crashed(msg) => bail!("{msg}"),
                super::SpawnedChildStatus::Running => {}
            }
        }
        if !noted_ready {
            eprintln!("[tenex-edge] waiting for daemon to answer RPCs...");
            noted_ready = true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("daemon socket exists but did not answer the handshake within 30s");
}

fn daemon_answers_ping() -> bool {
    matches!(try_call("ping", &serde_json::json!({})), Ok(Outcome::Ok(_)))
}

#[cfg(test)]
mod tests;
