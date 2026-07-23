//! ACP dialect method wrappers over [`RpcHandle`].
//!
//! ACP semantics: `session/prompt` resolves on `{stopReason}` (the response IS
//! turn completion), unlike app-server which signals completion via a separate
//! notification.

use std::path::Path;
use std::time::Duration;

use super::protocol::StopReason;
use super::transport::{RpcError, RpcHandle};

const RPC_TIMEOUT: Duration = Duration::from_secs(60);

/// ACP protocol methods over a live handle.
pub struct AcpClient {
    pub handle: RpcHandle,
}

impl AcpClient {
    pub fn new(handle: RpcHandle) -> Self {
        Self { handle }
    }

    /// `initialize {protocolVersion:1, clientCapabilities:{fs:{...}}}`.
    pub async fn initialize(&self) -> Result<serde_json::Value, RpcError> {
        self.handle
            .request_timeout(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": {
                        "fs": { "readTextFile": true, "writeTextFile": true }
                    }
                }),
                RPC_TIMEOUT,
            )
            .await
    }

    /// `session/new {cwd, mcpServers:[]}` -> sessionId. Claude's ACP adapter
    /// accepts a custom-agent selector in its namespaced metadata.
    pub async fn session_new(
        &self,
        cwd: &Path,
        claude_agent: Option<&str>,
    ) -> Result<String, RpcError> {
        let v = self
            .handle
            .request_timeout(
                "session/new",
                session_new_params(cwd, claude_agent),
                RPC_TIMEOUT,
            )
            .await?;
        extract_session_id(&v)
    }

    /// `session/load {sessionId, cwd, mcpServers:[]}` — cross-process resume.
    pub async fn session_load(&self, session_id: &str, cwd: &Path) -> Result<(), RpcError> {
        self.handle
            .request_timeout(
                "session/load",
                serde_json::json!({
                    "sessionId": session_id,
                    "cwd": cwd.to_string_lossy(),
                    "mcpServers": []
                }),
                RPC_TIMEOUT,
            )
            .await
            .map(|_| ())
    }

    /// `session/prompt {sessionId, prompt:[{type:text,text}]}` -> stopReason.
    pub async fn session_prompt(
        &self,
        session_id: &str,
        text: &str,
    ) -> Result<StopReason, RpcError> {
        // Once sent, a wall clock cannot prove whether the native turn is
        // still running. The correlated response, cancellation/child exit, or
        // transport failure is authoritative; this request is never replayed.
        let v = self
            .handle
            .request(
                "session/prompt",
                serde_json::json!({
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        let reason = v
            .get("stopReason")
            .and_then(|s| s.as_str())
            .map(StopReason::from_wire)
            .unwrap_or(StopReason::Other);
        Ok(reason)
    }

    /// `session/cancel {sessionId}` — notification (fire-and-forget).
    pub async fn session_cancel(&self, session_id: &str) {
        self.handle
            .notify(
                "session/cancel",
                serde_json::json!({ "sessionId": session_id }),
            )
            .await;
    }
}

fn session_new_params(cwd: &Path, claude_agent: Option<&str>) -> serde_json::Value {
    let mut params = serde_json::json!({
        "cwd": cwd.to_string_lossy(),
        "mcpServers": []
    });
    if let Some(agent) = claude_agent {
        params["_meta"] = serde_json::json!({
            "claudeCode": { "options": { "agent": agent } }
        });
    }
    params
}

fn extract_session_id(v: &serde_json::Value) -> Result<String, RpcError> {
    v.get("sessionId")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            RpcError::Protocol(super::protocol::RpcErrorObject {
                code: -1,
                message: "session/new response omitted required sessionId".to_string(),
                data: None,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_custom_agent_uses_adapter_metadata() {
        assert_eq!(
            session_new_params(Path::new("/work"), Some("reviewer")),
            serde_json::json!({
                "cwd": "/work",
                "mcpServers": [],
                "_meta": {
                    "claudeCode": { "options": { "agent": "reviewer" } }
                }
            })
        );
    }

    #[test]
    fn ordinary_acp_session_omits_adapter_metadata() {
        let params = session_new_params(Path::new("/work"), None);
        assert!(params.get("_meta").is_none());
    }
}
