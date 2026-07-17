//! `RpcTransport`: stdio JSON-RPC backend over `crate::rpc_harness`. Does NOT
//! use `src/pty/*`. ACP and app-server keep distinct persisted kinds while
//! sharing process/framing machinery. RPC has no unix socket, so liveness = child alive; the
//! per-session child lives in a process-global registry keyed by our endpoint
//! id.
//!
//! Lifecycle: `open`/`resume` spawn the child, register it, and start two tasks
//! per child — an update-drain task that folds the `session/update` stream into
//! [`AcpRuntime`] (transcript + running-turn tracking), and a reaper that awaits
//! child exit, then drops the registry entry and `wait()`s the zombie. Without
//! the reaper a self-exiting child leaks its entry and a zombie (the daemon
//! RAM-leak pattern), so it is not optional.

use super::{LaunchSpec, TransportKind};
use crate::rpc_harness::{
    spawn_config_from_driver, Callbacks, Dialect, RpcHandle, SessionUpdate, SpawnConfig,
};
use crate::session::Harness;
use anyhow::{Context, Result};
use tokio::sync::mpsc;

#[path = "acp/registry.rs"]
mod registry;
use registry::{register_child, registry};
#[path = "acp/native_agent.rs"]
mod native_agent;
#[path = "acp/open_session.rs"]
mod open_session;
#[path = "acp/session_transport.rs"]
mod session_transport;
#[path = "acp/thread_start_agent.rs"]
mod thread_start_agent;

pub struct RpcTransport {
    kind: TransportKind,
}

/// The outcome of opening (or resuming) an RPC-hosted session, before it is
/// wrapped into a `SessionEndpoint`.
pub struct AcpOpen {
    pub endpoint_id: String,
    pub native_id: String,
    pub pid: Option<u32>,
    /// The argv actually executed (recorded in session metadata; defect #8).
    pub argv: Vec<String>,
}

impl RpcTransport {
    pub(super) fn new(kind: TransportKind) -> Self {
        assert!(
            matches!(kind, TransportKind::Acp | TransportKind::AppServer),
            "RPC transport cannot host {kind:?}"
        );
        Self { kind }
    }

    fn dialect(&self) -> Dialect {
        match self.kind {
            TransportKind::Acp => Dialect::Acp,
            TransportKind::AppServer => Dialect::AppServer,
            TransportKind::Pty => unreachable!("RPC transport cannot host PTY"),
        }
    }

    pub(super) fn spawn_config(spec: &LaunchSpec, callbacks: Callbacks) -> Result<SpawnConfig> {
        let prepared = spec
            .prepared
            .rpc
            .as_ref()
            .context("RPC transport received no admitted launch plan")?;
        let mut cfg = spawn_config_from_driver(
            prepared.driver,
            &prepared.argv,
            &prepared.extra_env,
            std::path::PathBuf::from(&spec.abs_path),
            callbacks,
        )?;
        crate::session_host::agent_env::assign_launch(&mut cfg.env, &mut cfg.env_remove, spec);
        Ok(cfg)
    }

    /// Spawn from the immutable plan captured at admission. Configuration is not
    /// reloaded here: the executed runtime must exactly match the admitted facts.
    async fn spawn_child(
        &self,
        spec: &LaunchSpec,
    ) -> Result<(
        RpcHandle,
        Dialect,
        mpsc::UnboundedReceiver<SessionUpdate>,
        Vec<String>,
        Harness,
    )> {
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        let prepared = spec
            .prepared
            .rpc
            .as_ref()
            .context("RPC transport received no admitted launch plan")?;
        let argv = prepared.argv.clone();
        let harness = prepared.harness;
        let callbacks = Callbacks::allow_all(cwd.clone());
        let cfg = Self::spawn_config(spec, callbacks)?;
        let dialect = cfg.dialect;
        if dialect != self.dialect() {
            anyhow::bail!(
                "admitted {} transport resolved a {:?} RPC dialect",
                self.kind.as_str(),
                dialect
            );
        }
        let (handle, updates) = RpcHandle::spawn(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("spawning admitted RPC harness: {e}"))?;
        Ok((handle, dialect, updates, argv, harness))
    }

    fn endpoint_id(&self, slug: &str) -> String {
        // Must be unique across every concurrent session — two same-slug sessions
        // launched in the same wall-clock second would otherwise collide, silently
        // evicting one from the registry and mis-targeting its reaper (defect #1).
        // A process-global monotonic counter makes the id collision-free.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let now = crate::util::now_secs();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        format!(
            "{}-{slug}-{now}-{}-{seq}",
            self.kind.as_str(),
            std::process::id()
        )
    }

    fn synth_meta(
        spec: &LaunchSpec,
        endpoint_id: &str,
        pid: Option<u32>,
        argv: &[String],
    ) -> crate::pty::LaunchMetadata {
        crate::pty::LaunchMetadata {
            id: endpoint_id.to_string(),
            socket: String::new(),
            // ACP has no PTY supervisor. `supervisor_pid` is only a hint; a `0`
            // (no reported pid) is a deliberate sentinel — `pid_alive` treats it as
            // NOT live, and ACP session liveness is decided by the transport child
            // registry, not by pid (defect #3). Do NOT rely on this pid to prove an
            // ACP session live.
            supervisor_pid: pid.unwrap_or(0),
            agent: spec.slug.clone(),
            root: spec.root.clone(),
            cwd: spec.abs_path.clone(),
            ephemeral: spec.ephemeral,
            // Record the argv actually executed, not the nominal `base_command`
            // the ACP path never runs (defect #8).
            command: argv.to_vec(),
        }
    }

    /// Open a fresh session and register it; returns the open descriptor.
    pub async fn open(&self, spec: &LaunchSpec) -> Result<AcpOpen> {
        let (handle, dialect, updates, argv, harness) = self.spawn_child(spec).await?;
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        let native_id =
            open_session::open(&handle, dialect, &cwd, spec.native_agent.as_ref(), harness).await?;
        let endpoint_id = self.endpoint_id(&spec.slug);
        let pid = handle.pid;
        register_child(&endpoint_id, handle, native_id.clone(), cwd, updates);
        Ok(AcpOpen {
            endpoint_id,
            native_id,
            pid,
            argv,
        })
    }
}

#[cfg(test)]
#[path = "acp_reaper_tests.rs"]
mod acp_reaper_tests;
