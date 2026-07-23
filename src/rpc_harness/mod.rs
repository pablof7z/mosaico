//! Stdio JSON-RPC harness engine: the process-transport analogue of the Nostr
//! `transport`. Owns a child harness process and speaks its JSON-RPC dialect
//! (ACP or codex app-server) over newline-delimited JSON on stdio.
//!
//! New module; touches nothing under `src/identity*`. Serves the `RpcTurn` /
//! `AppServerSteer` capability rows the PTY path cannot.

pub mod acp;
pub mod app_server;
pub mod callbacks;
mod io_tasks;
pub mod protocol;
pub mod transport;

pub use acp::AcpClient;
pub use app_server::{AppServerClient, TurnFailure, TurnOutcome};
pub use callbacks::{Callbacks, FsBridge, PermissionPolicy};
pub use protocol::{Dialect, SessionUpdate, StopReason};
pub use transport::{RpcError, RpcHandle, SpawnConfig};

use crate::harness::{driver::EnvDirective, HarnessDriver};

/// Build a [`SpawnConfig`] from a resolved driver row + concrete argv.
///
/// `base_argv` is the driver's `base_argv` already extended with any profile
/// `extra_argv` (see `harness::ResolvedHarness::base_argv`). `dialect` is
/// inferred from the driver's transport.
pub fn spawn_config_from_driver(
    driver: &HarnessDriver,
    base_argv: &[String],
    extra_env: &[(String, String)],
    cwd: std::path::PathBuf,
    callbacks: Callbacks,
) -> anyhow::Result<SpawnConfig> {
    if base_argv.is_empty() {
        anyhow::bail!("empty base argv for harness transport");
    }
    let dialect = match driver.transport {
        crate::harness::Transport::Acp => Dialect::Acp,
        crate::harness::Transport::AppServer => Dialect::AppServer,
        other => anyhow::bail!(
            "transport {} is not an RPC transport (no stdio JSON-RPC dialect)",
            other.as_str()
        ),
    };

    let program = base_argv[0].clone();
    let args = base_argv[1..].to_vec();

    // Env hygiene from the daemon spawn intent + driver base_env.
    let mut env: Vec<(String, String)> = Vec::new();
    let mut env_remove: Vec<String> = Vec::new();
    for d in driver.base_env {
        match d {
            EnvDirective::Set(k, v) => env.push((k.to_string(), v.to_string())),
            EnvDirective::Remove(k) => env_remove.push(k.to_string()),
        }
    }
    env.extend(extra_env.iter().cloned());

    Ok(SpawnConfig {
        program,
        args,
        cwd,
        env,
        env_remove,
        dialect,
        callbacks,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
