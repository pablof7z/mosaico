//! `mosaico __acp-smoke <harness>` — debug driver that exercises the ACP /
//! app-server transport end-to-end without any agent.json / StoredKey wiring.
//!
//! For an ACP harness (Claude / Goose / OpenCode): initialize -> session/new ->
//! session/prompt("reply PONG") -> assert stopReason end_turn -> session/load
//! in a FRESH child process -> second session/prompt -> assert exact resume.
//!
//! For codex (app-server): initialize -> config/read -> thread/start ->
//! turn/start -> fresh process -> thread/resume -> second turn/start.

use anyhow::{Context, Result};
use clap::Args;

use crate::rpc_harness::{
    spawn_config_from_driver, AcpClient, AppServerClient, Callbacks, Dialect, RpcHandle, StopReason,
};

mod pi;

#[derive(Args)]
pub struct AcpSmokeArgs {
    /// Canonical harness name.
    pub harness: String,
    /// Working directory for the session (defaults to a temp dir).
    #[arg(long)]
    pub cwd: Option<String>,
    /// Prompt text to send.
    #[arg(long, default_value = "Reply with exactly one word: PONG")]
    pub prompt: String,
    /// Harness-native named profile to activate for both processes.
    #[arg(long)]
    pub profile: Option<String>,
    /// Optional named launch preset from presets.json.
    #[arg(long)]
    pub preset: Option<String>,
}

/// Resolve a canonical harness to its structured transport.
fn resolve_rpc(
    name: &str,
    profile: Option<&str>,
    preset: Option<&str>,
    scratch: &std::path::Path,
) -> Result<crate::harness::ResolvedHarness> {
    use crate::harness::{PresetsConfig, Transport};
    let harness = crate::session::Harness::from_str(name);
    if harness == crate::session::Harness::Unknown || harness.as_str() != name {
        anyhow::bail!("unknown canonical harness {name:?}");
    }
    let transport = match harness {
        crate::session::Harness::Codex => Transport::AppServer,
        crate::session::Harness::Pi => Transport::PiRpc,
        _ => Transport::Acp,
    };
    let presets = PresetsConfig::load()?;
    crate::harness::resolve_with(&presets, harness, transport, profile, preset, scratch)
}

pub async fn acp_smoke(args: AcpSmokeArgs) -> Result<()> {
    let cwd = match &args.cwd {
        Some(c) => std::path::PathBuf::from(c),
        None => std::env::temp_dir().join(format!("mosaico-acp-smoke-{}", std::process::id())),
    };
    std::fs::create_dir_all(&cwd).context("creating smoke cwd")?;

    let scratch = crate::config::mosaico_home()
        .join("harness-profiles")
        .join(&args.harness);
    let resolved = resolve_rpc(
        &args.harness,
        args.profile.as_deref(),
        args.preset.as_deref(),
        &scratch,
    )?;
    println!(
        "[acp-smoke] harness={} transport={} preset={}",
        resolved.harness.as_str(),
        resolved.transport.as_str(),
        resolved.preset.as_deref().unwrap_or("none")
    );
    println!(
        "[acp-smoke] argv={:?} cwd={}",
        resolved.base_argv,
        cwd.display()
    );

    resolved.profile.materialize()?;
    for (path, _) in &resolved.profile.files {
        println!("[acp-smoke] wrote profile file {}", path.display());
    }

    let mk_cfg = || {
        spawn_config_from_driver(
            resolved.driver,
            &resolved.base_argv,
            &resolved.profile.extra_env,
            cwd.clone(),
            Callbacks::allow_all(cwd.clone()),
        )
    };

    let cfg = mk_cfg()?;
    match cfg.dialect {
        Dialect::Acp => run_acp(cfg, &cwd, &args.prompt, mk_cfg).await,
        Dialect::AppServer => run_app_server(cfg, &cwd, &args.prompt, mk_cfg).await,
        Dialect::PiRpc => pi::run_pi_rpc(cfg, &args.prompt, mk_cfg).await,
    }
}

#[cfg(test)]
#[path = "acp_smoke/tests.rs"]
mod tests;

async fn run_acp(
    cfg: crate::rpc_harness::SpawnConfig,
    cwd: &std::path::Path,
    prompt: &str,
    mk_cfg: impl Fn() -> Result<crate::rpc_harness::SpawnConfig>,
) -> Result<()> {
    let (handle, mut updates) = RpcHandle::spawn(cfg)
        .await
        .map_err(|e| anyhow::anyhow!("spawning harness: {e}"))?;
    let chunks = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let c = chunks.clone();
    tokio::spawn(async move {
        while let Some(u) = updates.recv().await {
            if u.method.contains("update") {
                c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    });

    let client = AcpClient::new(handle.clone());
    client
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("initialize: {e}"))?;
    println!("[acp-smoke] initialize ok");
    let session_id = client
        .session_new(cwd, None)
        .await
        .map_err(|e| anyhow::anyhow!("session/new: {e}"))?;
    println!("[acp-smoke] session/new -> {session_id}");

    let stop = client
        .session_prompt(&session_id, prompt)
        .await
        .map_err(|e| anyhow::anyhow!("session/prompt: {e}"))?;
    println!(
        "[acp-smoke] session/prompt -> stopReason={:?} (chunks={})",
        stop,
        chunks.load(std::sync::atomic::Ordering::Relaxed)
    );
    if stop != StopReason::EndTurn {
        anyhow::bail!("expected stopReason end_turn, got {stop:?}");
    }
    handle.kill().await?;

    // Fresh process: cross-process resume.
    let (handle2, _u2) = RpcHandle::spawn(mk_cfg()?)
        .await
        .map_err(|e| anyhow::anyhow!("spawning resume harness: {e}"))?;
    let client2 = AcpClient::new(handle2.clone());
    client2
        .initialize()
        .await
        .map_err(|e| anyhow::anyhow!("initialize #2: {e}"))?;
    client2
        .session_load(&session_id, cwd)
        .await
        .map_err(|e| anyhow::anyhow!("session/load (cross-process resume): {e}"))?;
    println!("[acp-smoke] session/load cross-process resume ok for {session_id}");
    let resumed_stop = client2
        .session_prompt(&session_id, "Reply with exactly one word: RESUMED")
        .await
        .map_err(|e| anyhow::anyhow!("session/prompt after resume: {e}"))?;
    println!("[acp-smoke] resumed session/prompt -> stopReason={resumed_stop:?}");
    if resumed_stop != StopReason::EndTurn {
        anyhow::bail!("expected resumed stopReason end_turn, got {resumed_stop:?}");
    }
    handle2.kill().await?;

    println!("[acp-smoke] PASS");
    Ok(())
}

async fn run_app_server(
    cfg: crate::rpc_harness::SpawnConfig,
    cwd: &std::path::Path,
    prompt: &str,
    mk_cfg: impl Fn() -> Result<crate::rpc_harness::SpawnConfig>,
) -> Result<()> {
    let (handle, _updates) = RpcHandle::spawn(cfg)
        .await
        .map_err(|e| anyhow::anyhow!("spawning app-server: {e}"))?;
    let client = AppServerClient::new(handle.clone());
    client
        .initialize("mosaico", env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| anyhow::anyhow!("initialize: {e}"))?;
    println!("[acp-smoke] initialize ok");
    let effective = client
        .config_read(cwd)
        .await
        .map_err(|e| anyhow::anyhow!("config/read: {e}"))?;
    let catalog = client
        .model_catalog()
        .await
        .map_err(|e| anyhow::anyhow!("model/list: {e}"))?;
    let config = effective.get("config").cloned().unwrap_or_default();
    println!(
        "[acp-smoke] config/read -> model={} effort={} sandbox={} approval={}",
        config.get("model").unwrap_or(&serde_json::Value::Null),
        config
            .get("model_reasoning_effort")
            .unwrap_or(&serde_json::Value::Null),
        config
            .get("sandbox_mode")
            .unwrap_or(&serde_json::Value::Null),
        config
            .get("approval_policy")
            .unwrap_or(&serde_json::Value::Null),
    );
    let opened = client
        .thread_start(cwd, None, None)
        .await
        .map_err(|e| anyhow::anyhow!("thread/start: {e}"))?;
    catalog
        .admit(&opened)
        .map_err(|e| anyhow::anyhow!("launch admission: {e}"))?;
    println!(
        "[acp-smoke] admission ok -> model={} effort={}",
        opened.model,
        opened.reasoning_effort.as_deref().unwrap_or("<default>")
    );
    let thread_id = opened.thread_id;
    println!("[acp-smoke] thread/start -> {thread_id}");
    let outcome = client
        .turn_start(&thread_id, prompt)
        .await
        .map_err(|e| anyhow::anyhow!("turn/start: {e}"))?;
    println!("[acp-smoke] turn/completed -> {outcome}");
    handle.kill().await?;

    let (handle2, _updates2) = RpcHandle::spawn(mk_cfg()?)
        .await
        .map_err(|e| anyhow::anyhow!("spawning resume app-server: {e}"))?;
    let client2 = AppServerClient::new(handle2.clone());
    client2
        .initialize("mosaico", env!("CARGO_PKG_VERSION"))
        .await
        .map_err(|e| anyhow::anyhow!("initialize #2: {e}"))?;
    let resumed_catalog = client2
        .model_catalog()
        .await
        .map_err(|e| anyhow::anyhow!("model/list #2: {e}"))?;
    let resumed_opened = client2
        .thread_resume(&thread_id, cwd)
        .await
        .map_err(|e| anyhow::anyhow!("thread/resume: {e}"))?;
    resumed_catalog
        .admit(&resumed_opened)
        .map_err(|e| anyhow::anyhow!("resume admission: {e}"))?;
    println!("[acp-smoke] thread/resume cross-process ok for {thread_id}");
    let resumed = client2
        .turn_start(&thread_id, "Reply with exactly one word: RESUMED")
        .await
        .map_err(|e| anyhow::anyhow!("turn/start after resume: {e}"))?;
    println!("[acp-smoke] resumed turn/completed -> {resumed}");
    handle2.kill().await?;
    println!("[acp-smoke] PASS");
    Ok(())
}
