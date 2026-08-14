use super::{native_agent, thread_start_agent};
use crate::agent_catalog::NativeAgentActivation;
use crate::rpc_harness::{AcpClient, Dialect, PiRpcClient, RpcHandle};
use crate::session::Harness;
use anyhow::Result;
use std::path::Path;

pub(super) async fn open(
    handle: &RpcHandle,
    dialect: Dialect,
    cwd: &Path,
    activation: Option<&NativeAgentActivation>,
    harness: Harness,
) -> Result<String> {
    match dialect {
        Dialect::Acp => {
            let client = AcpClient::new(handle.clone());
            client
                .initialize()
                .await
                .map_err(|error| anyhow::anyhow!("ACP initialize: {error}"))?;
            client
                .session_new(cwd, native_agent::claude_selector(activation, harness))
                .await
                .map_err(|error| anyhow::anyhow!("ACP session/new: {error}"))
        }
        Dialect::AppServer => thread_start_agent::open(handle, cwd, activation).await,
        Dialect::PiRpc => PiRpcClient::new(handle.clone())
            .session_id()
            .await
            .map_err(|error| anyhow::anyhow!("Pi RPC get_state: {error}")),
    }
}
