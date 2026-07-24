//! Bootstrap-owned harness driver for the relay-assist modal.
//!
//! Spawns the resolved RPC harness child directly through the `rpc_harness`
//! engine (no daemon, no identity wiring — the `acp_smoke` pattern), runs one
//! assist turn, and forwards a normalized event stream plus human-routed
//! permission requests to the UI over channels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use serde_json::Value;

use super::decode::decode;
use super::harness::DeployTarget;
use super::transcript::DeployEvent;
use super::{PermissionAsk, PermissionOption};
use crate::rpc_harness::{
    spawn_config_from_driver, AcpClient, AppServerClient, Callbacks, Dialect, FsBridge,
    PermissionPolicy, RpcHandle, StopReason,
};

/// Live channels the UI drains while the driver runs.
pub(in crate::cli::install::onboarding) struct Driver {
    pub events: mpsc::Receiver<DeployEvent>,
    pub asks: mpsc::Receiver<PermissionAsk>,
    cancel: Arc<AtomicBool>,
}

impl Driver {
    /// Request teardown of the harness child.
    pub(in crate::cli::install::onboarding) fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// A driver with no backing task, for render/unit tests.
    #[cfg(test)]
    pub(in crate::cli::install::onboarding) fn disconnected() -> Driver {
        let (_e, events) = mpsc::channel();
        let (_a, asks) = mpsc::channel();
        Driver {
            events,
            asks,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Spawn the driver task and return its live channels immediately.
pub(in crate::cli::install::onboarding) fn start(
    handle: &tokio::runtime::Handle,
    target: DeployTarget,
    relay_url: String,
    owner_pubkey: String,
) -> Driver {
    let (event_tx, events) = mpsc::channel();
    let (ask_tx, asks) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_task = cancel.clone();

    handle.spawn(async move {
        if let Err(e) = run(
            target,
            relay_url,
            owner_pubkey,
            event_tx.clone(),
            ask_tx,
            cancel_task,
        )
        .await
        {
            let _ = event_tx.send(DeployEvent::Error(e.to_string()));
        }
    });

    Driver {
        events,
        asks,
        cancel,
    }
}

fn assist_prompt(relay_url: &str, owner_pubkey: &str) -> String {
    format!(
        "You are helping set up Mosaico on this machine. Mosaico coordinates \
         agents through a NIP-29 Nostr relay. Help me get a NIP-29 relay running \
         and reachable at {relay_url}.\n\n\
         Croissant is a good single-binary NIP-29 relay: it is configured with \
         the environment variables HOST, PORT, DATAPATH, and \
         OWNER_PUBLIC_KEY={owner_pubkey}. Use whatever approach fits this machine \
         (a local process, a container, or a service). Ask before installing \
         software or changing system state. Stop once the relay answers at \
         {relay_url}."
    )
}

async fn run(
    target: DeployTarget,
    relay_url: String,
    owner_pubkey: String,
    event_tx: mpsc::Sender<DeployEvent>,
    ask_tx: mpsc::Sender<PermissionAsk>,
    cancel: Arc<AtomicBool>,
) -> anyhow::Result<()> {
    let DeployTarget { resolved, cwd } = target;
    resolved.profile.materialize()?;

    let callbacks = Callbacks {
        permission: permission_policy(ask_tx),
        fs: FsBridge { root: cwd.clone() },
    };
    let cfg = spawn_config_from_driver(
        resolved.driver,
        &resolved.base_argv,
        &resolved.profile.extra_env,
        cwd.clone(),
        callbacks,
    )?;
    let dialect = cfg.dialect;

    let _ = event_tx.send(DeployEvent::Notice(format!(
        "starting {}…",
        resolved.harness.as_str()
    )));
    let (rpc, mut updates) = RpcHandle::spawn(cfg)
        .await
        .map_err(|e| anyhow::anyhow!("spawning harness: {e}"))?;

    // Forward decoded notifications to the transcript.
    let forward_tx = event_tx.clone();
    tokio::spawn(async move {
        while let Some(update) = updates.recv().await {
            if let Some(event) = decode(&update) {
                if forward_tx.send(event).is_err() {
                    break;
                }
            }
        }
    });

    // Tear the child down on cancel.
    let kill_handle = rpc.clone();
    tokio::spawn(async move {
        while !cancel.load(Ordering::Relaxed) {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
        let _ = kill_handle.kill().await;
    });

    let prompt = assist_prompt(&relay_url, &owner_pubkey);
    match dialect {
        Dialect::Acp => run_acp(&rpc, &cwd, &prompt, &event_tx).await,
        Dialect::AppServer => run_app_server(&rpc, &cwd, &prompt, &event_tx).await,
    }
}

async fn run_acp(
    rpc: &RpcHandle,
    cwd: &std::path::Path,
    prompt: &str,
    event_tx: &mpsc::Sender<DeployEvent>,
) -> anyhow::Result<()> {
    let client = AcpClient::new(rpc.clone());
    client
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("initialize: {e}"))?;
    let session_id = client
        .session_new(cwd, None)
        .await
        .map_err(|e| anyhow::anyhow!("session/new: {e}"))?;
    let stop = client
        .session_prompt(&session_id, prompt)
        .await
        .map_err(|e| anyhow::anyhow!("session/prompt: {e}"))?;
    let _ = event_tx.send(match stop {
        StopReason::EndTurn => DeployEvent::TurnEnded,
        other => DeployEvent::Notice(format!("agent stopped: {other:?}")),
    });
    Ok(())
}

async fn run_app_server(
    rpc: &RpcHandle,
    cwd: &std::path::Path,
    prompt: &str,
    event_tx: &mpsc::Sender<DeployEvent>,
) -> anyhow::Result<()> {
    let client = AppServerClient::new(rpc.clone());
    client
        .initialize("mosaico", env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| anyhow::anyhow!("initialize: {e}"))?;
    let _ = client.config_read(cwd).await;
    let catalog = client
        .model_catalog()
        .await
        .map_err(|e| anyhow::anyhow!("model catalog: {e}"))?;
    let opened = client
        .thread_start(cwd, None, None)
        .await
        .map_err(|e| anyhow::anyhow!("thread/start: {e}"))?;
    catalog
        .admit(&opened)
        .map_err(|e| anyhow::anyhow!("admission: {e}"))?;
    let outcome = client
        .turn_start(&opened.thread_id, prompt)
        .await
        .map_err(|e| anyhow::anyhow!("turn/start: {e}"))?;
    let _ = event_tx.send(DeployEvent::Notice(format!("agent turn: {outcome}")));
    let _ = event_tx.send(DeployEvent::TurnEnded);
    Ok(())
}

/// Build a permission policy that parks each request on the UI ask channel and
/// blocks (in the per-request task the engine spawns) until the human answers.
fn permission_policy(ask_tx: mpsc::Sender<PermissionAsk>) -> PermissionPolicy {
    let ask_tx = Arc::new(Mutex::new(ask_tx));
    PermissionPolicy::Custom(Arc::new(move |params: &Value| {
        let (summary, options) = parse_permission(params);
        if options.is_empty() {
            return None;
        }
        let (respond, response) = mpsc::channel();
        let ask = PermissionAsk {
            summary,
            options,
            respond,
        };
        if ask_tx.lock().ok()?.send(ask).is_err() {
            return None;
        }
        // Blocks only this request's task; the agent turn is paused meanwhile.
        response.recv().ok().flatten()
    }))
}

fn parse_permission(params: &Value) -> (String, Vec<PermissionOption>) {
    let summary = params
        .get("toolCall")
        .and_then(|t| t.get("title"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| "The agent is requesting permission".to_string());
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|opts| {
            opts.iter()
                .filter_map(|o| {
                    let id = o.get("optionId").and_then(Value::as_str)?.to_string();
                    let kind = o.get("kind").and_then(Value::as_str).unwrap_or("");
                    let label = o
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    Some(PermissionOption {
                        id,
                        label,
                        allow: kind.starts_with("allow"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (summary, options)
}
