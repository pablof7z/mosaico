//! Pi's native strict-JSONL RPC dialect.

use std::time::Duration;

use super::protocol::PI_TURN_KEY;
use super::transport::{RpcError, RpcHandle, TurnSignal};

const RPC_TIMEOUT: Duration = Duration::from_secs(60);

pub struct PiRpcClient {
    handle: RpcHandle,
}

impl PiRpcClient {
    pub fn new(handle: RpcHandle) -> Self {
        Self { handle }
    }

    pub async fn session_id(&self) -> Result<String, RpcError> {
        let state = self
            .handle
            .pi_request_timeout(serde_json::json!({"type":"get_state"}), RPC_TIMEOUT)
            .await?;
        state
            .get("sessionId")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
            .ok_or_else(|| protocol_error("Pi get_state omitted required sessionId"))
    }

    /// Prompt acceptance is not completion. Register before sending, then wait
    /// for the authoritative `agent_settled` event emitted by this process.
    pub async fn prompt(&self, text: &str) -> Result<(), RpcError> {
        let mut observer = self.handle.register_turn_waiter(PI_TURN_KEY)?;
        self.handle
            .pi_request(serde_json::json!({"type":"prompt", "message":text}))
            .await?;
        loop {
            match observer.recv().await {
                Some(TurnSignal::Completed(_)) => return Ok(()),
                Some(TurnSignal::Reconcile) => {}
                None => return Err(RpcError::ChildExited),
            }
        }
    }

    pub async fn steer(&self, text: &str) -> Result<(), RpcError> {
        self.handle
            .pi_request_timeout(
                serde_json::json!({"type":"steer", "message":text}),
                RPC_TIMEOUT,
            )
            .await
            .map(|_| ())
    }

    pub async fn abort(&self) {
        let _ = self
            .handle
            .pi_request_timeout(serde_json::json!({"type":"abort"}), RPC_TIMEOUT)
            .await;
    }
}

fn protocol_error(message: impl Into<String>) -> RpcError {
    RpcError::Protocol(super::protocol::RpcErrorObject {
        code: -1,
        message: message.into(),
        data: None,
    })
}
