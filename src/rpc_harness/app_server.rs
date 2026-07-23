//! Codex `app-server` dialect wrappers over [`RpcHandle`].
//!
//! Key asymmetry vs ACP: `turn/start` does NOT resolve on its JSON-RPC
//! response; the turn completes via a separate `turn/completed` notification.
//! `turn_start` registers an observer before sending, then accepts only the
//! current generated protocol's nested `turn.status`. Periodic `thread/read`
//! reconciles a lost terminal notification without inventing a timeout result.

use std::path::Path;
use std::time::Duration;

use super::transport::{RpcError, RpcHandle};

mod outcome;
mod start;
mod turn_protocol;
pub use outcome::{TurnFailure, TurnOutcome};
use turn_protocol::{parse_thread_started, parse_turn_baseline};

const RPC_TIMEOUT: Duration = Duration::from_secs(60);

pub struct AppServerClient {
    pub handle: RpcHandle,
}

impl AppServerClient {
    pub fn new(handle: RpcHandle) -> Self {
        Self { handle }
    }

    /// `initialize {clientInfo:{name,title,version}}`.
    pub async fn initialize(
        &self,
        name: &str,
        version: &str,
    ) -> Result<serde_json::Value, RpcError> {
        self.handle
            .request_timeout(
                "initialize",
                serde_json::json!({
                    "clientInfo": { "name": name, "title": name, "version": version }
                }),
                RPC_TIMEOUT,
            )
            .await
    }

    /// `config/read {cwd,includeLayers:true}` -> effective process config.
    pub async fn config_read(&self, cwd: &Path) -> Result<serde_json::Value, RpcError> {
        self.handle
            .request_timeout(
                "config/read",
                serde_json::json!({
                    "cwd": cwd.to_string_lossy(),
                    "includeLayers": true
                }),
                RPC_TIMEOUT,
            )
            .await
    }

    /// Start a thread, optionally applying a resolved Codex custom agent at the
    /// app-server's native instruction/config boundary.
    pub async fn thread_start(
        &self,
        cwd: &Path,
        developer_instructions: Option<&str>,
        config: Option<&serde_json::Value>,
    ) -> Result<String, RpcError> {
        let value = self
            .handle
            .request_timeout(
                "thread/start",
                thread_start_params(cwd, developer_instructions, config),
                RPC_TIMEOUT,
            )
            .await?;
        let (thread_id, baseline) = parse_thread_started(value)?;
        self.handle.record_turn_baseline(&thread_id, baseline);
        Ok(thread_id)
    }

    /// `turn/steer {threadId, expectedTurnId, input}` — mid-turn inject; resolves
    /// on its own response.
    pub async fn turn_steer(
        &self,
        thread_id: &str,
        expected_turn_id: &str,
        text: &str,
    ) -> Result<(), RpcError> {
        self.handle
            .request_timeout(
                "turn/steer",
                serde_json::json!({
                    "threadId": thread_id,
                    "expectedTurnId": expected_turn_id,
                    "input": [{ "type": "text", "text": text }]
                }),
                RPC_TIMEOUT,
            )
            .await
            .map(|_| ())
    }

    /// `thread/resume {threadId, cwd}`.
    pub async fn thread_resume(&self, thread_id: &str, cwd: &Path) -> Result<(), RpcError> {
        let value = self
            .handle
            .request_timeout(
                "thread/resume",
                serde_json::json!({
                    "threadId": thread_id,
                    "cwd": cwd.to_string_lossy()
                }),
                RPC_TIMEOUT,
            )
            .await?;
        let baseline = parse_turn_baseline(thread_id, value)?;
        self.handle.record_turn_baseline(thread_id, baseline);
        Ok(())
    }
}

fn thread_start_params(
    cwd: &Path,
    developer_instructions: Option<&str>,
    config: Option<&serde_json::Value>,
) -> serde_json::Value {
    let mut params = serde_json::json!({ "cwd": cwd.to_string_lossy() });
    let object = params.as_object_mut().expect("thread/start params object");
    if let Some(instructions) = developer_instructions {
        object.insert(
            "developerInstructions".to_string(),
            serde_json::Value::String(instructions.to_string()),
        );
    }
    if let Some(config) = config {
        object.insert("config".to_string(), config.clone());
    }
    params
}

#[cfg(test)]
#[path = "app_server/tests.rs"]
mod tests;
