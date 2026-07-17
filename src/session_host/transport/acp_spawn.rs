//! Fire-and-forget turn helpers for [`super::acp::RpcTransport`]: spawn an ACP
//! `session/prompt`, an app-server `turn/start`, or an app-server `turn/steer`
//! (immediate or gated on a not-yet-known turn id). Extracted from `acp.rs` to
//! keep that module under the size ceiling; the deliver path calls these.

use std::sync::{Arc, Mutex};

use crate::rpc_harness::{AcpClient, AppServerClient, RpcHandle};

use super::acp_runtime::{AcpRuntime, SteerState};

pub(crate) fn spawn_acp_prompt(
    handle: RpcHandle,
    native_id: String,
    text: String,
    runtime: Arc<Mutex<AcpRuntime>>,
) {
    tokio::spawn(async move {
        let res = AcpClient::new(handle)
            .session_prompt(&native_id, &text)
            .await;
        if let Ok(mut rt) = runtime.lock() {
            rt.mark_turn_finished();
        }
        if let Err(e) = res {
            tracing::warn!(session = %native_id, "ACP session/prompt failed: {e}");
        }
    });
}

pub(crate) fn spawn_app_server_turn(
    handle: RpcHandle,
    native_id: String,
    text: String,
    runtime: Arc<Mutex<AcpRuntime>>,
) {
    tokio::spawn(async move {
        let res = AppServerClient::new(handle)
            .turn_start(&native_id, &text)
            .await;
        if let Ok(mut rt) = runtime.lock() {
            rt.mark_turn_finished();
        }
        if let Err(e) = res {
            tracing::warn!(thread = %native_id, "app-server turn/start failed: {e}");
        }
    });
}

pub(crate) fn spawn_app_server_steer(
    handle: RpcHandle,
    native_id: String,
    turn_id: String,
    text: String,
) {
    tokio::spawn(async move {
        if let Err(e) = AppServerClient::new(handle)
            .turn_steer(&native_id, &turn_id, &text)
            .await
        {
            tracing::warn!(thread = %native_id, turn = %turn_id, "app-server turn/steer failed: {e}");
        }
    });
}

/// How long to wait for a running turn's id to arrive before giving up on a
/// gated steer. The id lands on the first `session/update` of the turn, so this
/// only ever waits out the RPC round-trip; the cap just bounds a stuck child.
const STEER_GATE_TIMEOUT_MS: u64 = 5_000;
/// Poll cadence while waiting for the turn id.
const STEER_GATE_POLL_MS: u64 = 50;

/// Defect #2: a steer arrived while a turn is running but its id is not yet
/// known. Rather than start a second concurrent turn, wait (bounded) for the id
/// to be observed on the update stream, then steer the real turn. If the turn
/// ends or the id never arrives, drop the steer with a `WARN` — never fabricate a
/// concurrent turn.
pub(crate) fn spawn_app_server_steer_pending(
    handle: RpcHandle,
    native_id: String,
    text: String,
    runtime: Arc<Mutex<AcpRuntime>>,
) {
    tokio::spawn(async move {
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(STEER_GATE_TIMEOUT_MS);
        loop {
            let state = runtime.lock().ok().map(|rt| rt.steer_state());
            match state {
                Some(SteerState::Ready(turn_id)) => {
                    if let Err(e) = AppServerClient::new(handle)
                        .turn_steer(&native_id, &turn_id, &text)
                        .await
                    {
                        tracing::warn!(thread = %native_id, turn = %turn_id, "app-server gated turn/steer failed: {e}");
                    }
                    return;
                }
                Some(SteerState::Idle) | None => {
                    tracing::warn!(thread = %native_id, "steer target ended before its turn id was known; dropping steer");
                    return;
                }
                Some(SteerState::AwaitingId) => {
                    if std::time::Instant::now() >= deadline {
                        tracing::warn!(thread = %native_id, "timed out waiting for turn id; dropping steer to avoid a second concurrent turn");
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(STEER_GATE_POLL_MS)).await;
                }
            }
        }
    });
}
