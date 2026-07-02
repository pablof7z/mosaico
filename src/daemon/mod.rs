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
