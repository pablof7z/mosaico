//! The three background tasks that frame a child harness's stdio: writer,
//! stderr drain, and the single reader that owns the correlation map + inbound
//! dispatch. Split out of `transport.rs` to keep each file within the repo's
//! file-size doctrine.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch};

use super::callbacks::Callbacks;
use super::protocol::{classify, Inbound, RpcErrorObject, SessionUpdate};
use super::transport::{AppServerRouting, PendingMap, TurnSignal};

pub(super) async fn writer_task(
    mut stdin: tokio::process::ChildStdin,
    mut rx: mpsc::Receiver<String>,
) {
    while let Some(mut line) = rx.recv().await {
        line.push('\n');
        if stdin.write_all(line.as_bytes()).await.is_err() {
            break;
        }
        let _ = stdin.flush().await;
    }
}

pub(super) async fn stderr_task(stderr: tokio::process::ChildStderr, program: String) {
    let mut reader = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = reader.next_line().await {
        tracing::debug!(target: "rpc_harness", program = %program, "stderr: {line}");
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn reader_task(
    stdout: tokio::process::ChildStdout,
    pending: PendingMap,
    app_server_routing: AppServerRouting,
    write_tx: mpsc::Sender<String>,
    update_tx: mpsc::UnboundedSender<SessionUpdate>,
    callbacks: Callbacks,
    alive: Arc<AtomicBool>,
    exit_tx: watch::Sender<bool>,
) {
    let mut reader = BufReader::new(stdout).lines();
    // `while let Ok(Some(..))` exits on both EOF (`Ok(None)`) and read error.
    while let Ok(Some(line)) = reader.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        dispatch_inbound(
            classify(value),
            &pending,
            &app_server_routing,
            &write_tx,
            &update_tx,
            &callbacks,
        )
        .await;
    }
    // Child stream closed: mark dead + fail all pending waiters.
    alive.store(false, Ordering::Relaxed);
    // Signal the reaper that the child has exited so it can `wait()` the zombie
    // and drop the process-global registry entry. Ignore a closed receiver.
    let _ = exit_tx.send(true);
    let drained: Vec<_> = pending.lock().unwrap().drain().collect();
    for (_, tx) in drained {
        let _ = tx.send(Err(super::transport::RpcError::ChildExited));
    }
    app_server_routing.clear_waiters();
}

async fn dispatch_inbound(
    inbound: Inbound,
    pending: &PendingMap,
    app_server_routing: &AppServerRouting,
    write_tx: &mpsc::Sender<String>,
    update_tx: &mpsc::UnboundedSender<SessionUpdate>,
    callbacks: &Callbacks,
) {
    match inbound {
        Inbound::Response { id, result } => {
            if let Some(tx) = pending.lock().unwrap().remove(&id) {
                let _ = tx.send(result.map_err(super::transport::RpcError::Protocol));
            }
        }
        Inbound::Request { id, method, params } => {
            handle_agent_request(id, method, params, write_tx.clone(), callbacks.clone());
        }
        Inbound::Notification { method, params } => {
            if method == "turn/completed" {
                signal_turn(
                    app_server_routing,
                    &params,
                    TurnSignal::Completed(params.clone()),
                );
            } else if method == "thread/status/changed" {
                signal_turn(app_server_routing, &params, TurnSignal::Reconcile);
            }
            // Unbounded, non-blocking send: never drops an update (defect #15), and
            // cannot deadlock the reader (no backpressure). A closed receiver (drain
            // task gone) is the only error and is safely ignored.
            let _ = update_tx.send(SessionUpdate { method, params });
        }
        Inbound::Other => {}
    }
}

fn signal_turn(
    app_server_routing: &AppServerRouting,
    params: &serde_json::Value,
    signal: TurnSignal,
) {
    let Some(thread_id) = params.get("threadId").and_then(serde_json::Value::as_str) else {
        tracing::warn!("app-server lifecycle notification omitted required threadId");
        return;
    };
    app_server_routing.signal(thread_id, signal);
}

/// Handle an agent->client request in a spawned task so slow fs IO never stalls
/// the reader loop; reply flows back through the shared write channel.
fn handle_agent_request(
    id: serde_json::Value,
    method: String,
    params: serde_json::Value,
    write_tx: mpsc::Sender<String>,
    callbacks: Callbacks,
) {
    tokio::spawn(async move {
        let result: Result<serde_json::Value, RpcErrorObject> = match method.as_str() {
            "session/request_permission" => match callbacks.permission.choose(&params) {
                Some(option_id) => Ok(serde_json::json!({
                    "outcome": { "outcome": "selected", "optionId": option_id }
                })),
                None => Ok(serde_json::json!({ "outcome": { "outcome": "cancelled" } })),
            },
            "fs/read_text_file" => callbacks.fs.read_text(&params).await,
            "fs/write_text_file" => callbacks.fs.write_text(&params).await,
            _ => Err(RpcErrorObject {
                code: -32601,
                message: format!("method not handled: {method}"),
                data: None,
            }),
        };
        let reply = match result {
            Ok(v) => serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": v }),
            Err(e) => serde_json::json!({
                "jsonrpc": "2.0", "id": id,
                "error": { "code": e.code, "message": e.message }
            }),
        };
        let _ = write_tx.send(reply.to_string()).await;
    });
}
