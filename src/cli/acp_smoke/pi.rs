use anyhow::Result;

use crate::rpc_harness::{PiRpcClient, RpcHandle, SpawnConfig};

pub(super) async fn run_pi_rpc(
    cfg: SpawnConfig,
    prompt: &str,
    mk_cfg: impl Fn() -> Result<SpawnConfig>,
) -> Result<()> {
    let (handle, _updates) = RpcHandle::spawn(cfg)
        .await
        .map_err(|error| anyhow::anyhow!("spawning Pi RPC: {error}"))?;
    let client = PiRpcClient::new(handle.clone());
    let session_id = client
        .session_id()
        .await
        .map_err(|error| anyhow::anyhow!("Pi get_state: {error}"))?;
    client
        .prompt(prompt)
        .await
        .map_err(|error| anyhow::anyhow!("Pi prompt: {error}"))?;
    handle.kill().await?;

    let mut resume_cfg = mk_cfg()?;
    resume_cfg
        .args
        .extend(["--session".into(), session_id.clone()]);
    let (resumed_handle, _updates) = RpcHandle::spawn(resume_cfg)
        .await
        .map_err(|error| anyhow::anyhow!("spawning resumed Pi RPC: {error}"))?;
    let resumed = PiRpcClient::new(resumed_handle.clone());
    let resumed_id = resumed
        .session_id()
        .await
        .map_err(|error| anyhow::anyhow!("Pi resumed get_state: {error}"))?;
    if resumed_id != session_id {
        anyhow::bail!("Pi resumed a different session: expected {session_id}, got {resumed_id}");
    }
    resumed
        .prompt("Reply with exactly one word: RESUMED")
        .await
        .map_err(|error| anyhow::anyhow!("Pi resumed prompt: {error}"))?;
    resumed_handle.kill().await?;
    println!("[acp-smoke] PASS Pi RPC session {session_id}");
    Ok(())
}
