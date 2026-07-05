//! The per-machine daemon (M1 daemon migration, stages 2 & 3).
//!
//! ONE daemon per machine is the sole owner of `state.db`, the (single) relay
//! connection, the inbox, presence, membership cache, and peer pruning. Every CLI
//! invocation and every per-session engine becomes a thin client that talks to
//! the daemon over a Unix domain socket. One writer by construction → the
//! multi-writer corruption window goes to zero; N relay connections collapse to
//! one. See `docs/daemon-design.md`.

pub mod blocking;
pub mod client;
pub mod protocol;
pub mod server;
pub(crate) mod storage_paths;
pub mod tail_event;

mod spawn;

use crate::config;
use std::path::PathBuf;

/// `$TENEX_EDGE_HOME/daemon.sock` — the UDS the daemon binds and clients connect.
pub fn socket_path() -> PathBuf {
    config::edge_home().join("daemon.sock")
}

/// `$TENEX_EDGE_HOME/daemon.lock` — `flock`'d to serialize racing spawners and
/// to mark "a daemon is running".
pub fn lock_path() -> PathBuf {
    config::edge_home().join("daemon.lock")
}

/// `$TENEX_EDGE_HOME/daemon.log` — detached daemon stdout+stderr.
pub fn log_path() -> PathBuf {
    config::edge_home().join("daemon.log")
}

/// Last few lines of `daemon.log`, for surfacing a just-crashed daemon's own
/// error (e.g. a missing config.json) alongside a generic startup-timeout
/// failure instead of leaving the caller to go dig for it. Best-effort:
/// unreadable/absent log yields a placeholder rather than propagating a
/// second error.
pub(crate) fn tail_daemon_log() -> String {
    const MAX_LINES: usize = 20;
    match std::fs::read_to_string(log_path()) {
        Ok(contents) => {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(MAX_LINES);
            lines[start..].join("\n")
        }
        Err(e) => format!("(could not read daemon.log: {e})"),
    }
}

/// Outcome of polling a daemon `Child` this process just spawned.
pub(crate) enum SpawnedChildStatus {
    /// Still running — keep polling for the handshake.
    Running,
    /// Exited successfully: `lifecycle::run` does this when another daemon
    /// already held the startup lock, so this is a lost spawn race, not a
    /// crash. The caller should stop watching this child and keep polling —
    /// the winning daemon should still come up.
    LostRace,
    /// Exited with a failure — the message includes the `daemon.log` tail.
    Crashed(String),
}

/// Non-blocking check of a spawned daemon child, shared by the sync and async
/// spawn-if-absent paths so a crash is reported immediately instead of making
/// the caller wait out the full startup timeout.
pub(crate) fn poll_spawned_child(child: &mut std::process::Child) -> SpawnedChildStatus {
    match child.try_wait() {
        Ok(Some(status)) if status.success() => SpawnedChildStatus::LostRace,
        Ok(Some(status)) => SpawnedChildStatus::Crashed(format!(
            "daemon exited immediately ({status}); last daemon.log lines:\n{}",
            tail_daemon_log()
        )),
        _ => SpawnedChildStatus::Running,
    }
}

pub fn store_path() -> PathBuf {
    config::edge_home().join("state.db")
}

/// `$TENEX_EDGE_HOME/daemon.inhibit` — presence of this file tells hook-path
/// callers not to spawn the daemon. Written by `tenex-edge stop`; removed the
/// next time any non-hook command (who, chat, tail, …) needs the daemon.
pub fn inhibit_path() -> PathBuf {
    config::edge_home().join("daemon.inhibit")
}

pub fn is_inhibited() -> bool {
    inhibit_path().exists()
}

pub fn set_inhibit() {
    let _ = std::fs::write(inhibit_path(), "");
}

pub fn clear_inhibit() {
    let _ = std::fs::remove_file(inhibit_path());
}
