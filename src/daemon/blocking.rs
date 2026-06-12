//! A synchronous (blocking) thin client for CLI verbs that are NOT `async`.
//!
//! The async [`super::client::Client`] can't be `block_on`'d from inside the
//! tokio runtime the CLI already runs on (that panics). Sync verbs instead use
//! this plain `std::os::unix::net::UnixStream` client: spawn-if-absent via the
//! same `connect_or_spawn` mechanics is async, so for the blocking path we do a
//! minimal connect-or-spawn against the socket directly. Blocking I/O on a
//! one-shot CLI invocation is fine.

use super::protocol::{protocol_version, PleaseExit};
use super::{lock_path, log_path, socket_path};
use crate::config;
use anyhow::{bail, Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

/// Connect to the daemon (spawning if absent), do the handshake, send one
/// request, and return the `ok` payload. Mirrors the async client's behavior
/// including the version-skew exit+respawn, but synchronously.
pub fn call(method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
    for _ in 0..5 {
        match try_call(method, &params) {
            Ok(Outcome::Ok(v)) => return Ok(v),
            Ok(Outcome::Err(code, msg)) => bail!("daemon error [{code}]: {msg}"),
            Ok(Outcome::SkewExit) => {
                std::thread::sleep(Duration::from_millis(200));
                spawn_if_absent()?;
            }
            Err(e) => {
                if e.to_string().contains("is newer than this binary") {
                    return Err(e);
                }
                spawn_if_absent()?;
            }
        }
    }
    bail!("could not complete daemon call {method}")
}

enum Outcome {
    Ok(serde_json::Value),
    Err(String, String),
    SkewExit,
}

fn try_call(method: &str, params: &serde_json::Value) -> Result<Outcome> {
    let stream = UnixStream::connect(socket_path()).context("connecting to daemon socket")?;
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);

    // hello → welcome.
    writeln!(
        w,
        "{}",
        serde_json::json!({"protocol": protocol_version(), "client_version": env!("CARGO_PKG_VERSION")})
    )?;
    let mut welcome_line = String::new();
    if r.read_line(&mut welcome_line)? == 0 {
        bail!("daemon closed before welcome");
    }
    let welcome: serde_json::Value = serde_json::from_str(welcome_line.trim())?;
    let dproto = welcome["protocol"].as_u64().unwrap_or(0) as u32;
    if dproto < protocol_version() {
        // Older daemon under a newer binary: ask it to exit, then respawn.
        writeln!(
            w,
            "{}",
            serde_json::to_string(&PleaseExit {
                protocol: protocol_version()
            })?
        )?;
        let _ = w.flush();
        return Ok(Outcome::SkewExit);
    }
    if dproto > protocol_version() {
        let mine = protocol_version();
        bail!(
            "daemon protocol {dproto} is newer than this binary's {mine} — restart your tenex-edge session (or reinstall)"
        );
    }

    // request → response.
    writeln!(
        w,
        "{}",
        serde_json::json!({"id": 1, "method": method, "params": params})
    )?;
    let mut resp_line = String::new();
    if r.read_line(&mut resp_line)? == 0 {
        bail!("daemon closed the connection");
    }
    let resp: serde_json::Value = serde_json::from_str(resp_line.trim())?;
    if let Some(err) = resp.get("error") {
        let code = err["code"].as_str().unwrap_or("error").to_string();
        let msg = err["message"].as_str().unwrap_or("").to_string();
        return Ok(Outcome::Err(code, msg));
    }
    Ok(Outcome::Ok(
        resp.get("ok").cloned().unwrap_or(serde_json::Value::Null),
    ))
}

/// Synchronous spawn-if-absent: under the startup `flock`, reclaim a stale
/// socket and spawn a detached daemon, then poll-connect.
fn spawn_if_absent() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;
    let lock = super::client::StartupLock::acquire()?;
    if UnixStream::connect(socket_path()).is_ok() {
        return Ok(());
    }
    let sock = socket_path();
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    spawn_detached_daemon()?;
    drop(lock);

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if UnixStream::connect(socket_path()).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    bail!("daemon did not come up");
}

fn spawn_detached_daemon() -> Result<()> {
    let exe = match std::env::var_os("TENEX_EDGE_BIN") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe().context("locating own executable")?,
    };
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .context("opening daemon.log")?;
    let log_err = log.try_clone()?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("__daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.spawn().context("spawning detached daemon")?;
    Ok(())
}

/// `lock_path` re-export silencer (kept for symmetry with the async client).
#[allow(dead_code)]
fn _lock_path_ref() -> std::path::PathBuf {
    lock_path()
}
