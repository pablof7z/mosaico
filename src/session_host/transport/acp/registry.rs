//! Process-global ACP child registry and exit reaper.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::mpsc;

use super::super::acp_runtime::AcpRuntime;
use crate::rpc_harness::{RpcHandle, SessionUpdate};
use crate::session_host::transport::TransportKind;

/// A live ACP/app-server child plus its native session token.
pub(super) struct AcpChild {
    pub(super) handle: RpcHandle,
    /// ACP `sessionId` or app-server `threadId`.
    pub(super) native_id: String,
    /// Captured transcript + running-turn state, fed by the update-drain task.
    pub(super) runtime: Arc<Mutex<AcpRuntime>>,
}

pub(super) fn registry() -> &'static Mutex<HashMap<String, AcpChild>> {
    static REG: OnceLock<Mutex<HashMap<String, AcpChild>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Commit registry removal only after the transport has confirmed process
/// exit. Keeping this as one reducer makes it impossible for a failed kill to
/// orphan a still-owned child merely because teardown was requested.
pub(super) fn remove_after_exit_confirmation(
    endpoint_id: &str,
    confirmation: std::io::Result<()>,
) -> std::io::Result<()> {
    confirmation?;
    registry().lock().unwrap().remove(endpoint_id);
    Ok(())
}

/// Register a freshly-opened child, drain its updates into shared runtime state,
/// and reap both the registry entry and zombie when it exits.
pub(super) fn register_child(
    endpoint_id: &str,
    handle: RpcHandle,
    native_id: String,
    _cwd: std::path::PathBuf,
    mut updates: mpsc::UnboundedReceiver<SessionUpdate>,
) {
    let runtime = Arc::new(Mutex::new(AcpRuntime::default()));
    let rt_updates = runtime.clone();
    tokio::spawn(async move {
        while let Some(update) = updates.recv().await {
            if let Ok(mut runtime) = rt_updates.lock() {
                runtime.note_update(&update.method, &update.params);
            }
        }
    });

    let reaper_handle = handle.clone();
    let reaper_id = endpoint_id.to_string();
    tokio::spawn(async move {
        reaper_handle.wait_exit().await;
        registry().lock().unwrap().remove(&reaper_id);
        tracing::debug!(endpoint = %reaper_id, "acp child exited; registry entry reaped");
    });
    registry().lock().unwrap().insert(
        endpoint_id.to_string(),
        AcpChild {
            handle,
            native_id,
            runtime,
        },
    );
}

/// Explicitly terminate every RPC process group before the daemon releases its
/// in-memory registry. Stdio RPC sessions cannot be re-adopted by a replacement
/// daemon, so orderly shutdown must never abandon them as unowned processes.
pub(super) async fn shutdown_all() -> Vec<(TransportKind, String, std::io::Result<()>)> {
    let owned = registry()
        .lock()
        .unwrap()
        .iter()
        .map(|(endpoint, child)| {
            let kind = match child.handle.dialect {
                crate::rpc_harness::Dialect::Acp => TransportKind::Acp,
                crate::rpc_harness::Dialect::AppServer => TransportKind::AppServer,
                crate::rpc_harness::Dialect::PiRpc => TransportKind::PiRpc,
            };
            (kind, endpoint.clone(), child.handle.clone())
        })
        .collect::<Vec<_>>();
    let mut kills = tokio::task::JoinSet::new();
    for (kind, endpoint, handle) in owned {
        kills.spawn(async move { (kind, endpoint, handle.kill().await) });
    }
    let mut results = Vec::with_capacity(kills.len());
    while let Some(joined) = kills.join_next().await {
        let (kind, endpoint, confirmation) = match joined {
            Ok(result) => result,
            Err(error) => {
                tracing::error!(%error, "RPC shutdown task failed");
                continue;
            }
        };
        if confirmation.is_ok() {
            registry().lock().unwrap().remove(&endpoint);
        }
        results.push((kind, endpoint, confirmation));
    }
    results
}
