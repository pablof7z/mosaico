//! `AcpTransport`: stdio JSON-RPC backend over `crate::rpc_harness`. Does NOT
//! use `src/pty/*`. ACP has no unix socket, so liveness = child alive; the
//! per-session child lives in a process-global registry keyed by our endpoint
//! id.
//!
//! Lifecycle: `open`/`resume` spawn the child, register it, and start two tasks
//! per child — an update-drain task that folds the `session/update` stream into
//! [`AcpRuntime`] (transcript + running-turn tracking), and a reaper that awaits
//! child exit, then drops the registry entry and `wait()`s the zombie. Without
//! the reaper a self-exiting child leaks its entry and a zombie (the daemon
//! RAM-leak pattern), so it is not optional.

use super::acp_runtime::SteerState;
use super::acp_spawn::{
    spawn_acp_prompt, spawn_app_server_steer, spawn_app_server_steer_pending, spawn_app_server_turn,
};
use super::{
    EndpointRef, LaunchSpec, ResumeSpec, SessionEndpoint, SessionTransport, TransportKind,
};
use crate::harness::{self, config::HarnessesConfig, Transport};
use crate::rpc_harness::{
    spawn_config_from_driver, AcpClient, AppServerClient, Callbacks, Dialect, RpcHandle,
    SessionUpdate,
};
use crate::session::Harness;
use anyhow::{Context, Result};
use tokio::sync::mpsc;

#[path = "acp/registry.rs"]
mod registry;
use registry::{register_child, registry};
#[path = "acp/thread_start_agent.rs"]
mod thread_start_agent;

pub struct AcpTransport;

/// The harness BUNDLE name to resolve a spec's driver from — never the agent slug
/// (defect #1). An agent `reviewer` may carry bundle `codex-acp`, and `reviewer`
/// is not a `harnesses.json` key.
pub(crate) fn bundle_name(spec: &LaunchSpec) -> &str {
    &spec.bundle
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

impl AcpTransport {
    /// Resolve the harness bundle for `spec.bundle` and spawn its RPC child,
    /// returning the live handle + dialect + the update stream + the argv actually
    /// executed. Shared by launch/resume.
    async fn spawn_child(
        spec: &LaunchSpec,
    ) -> Result<(
        RpcHandle,
        Dialect,
        mpsc::UnboundedReceiver<SessionUpdate>,
        Vec<String>,
    )> {
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        let bundle = bundle_name(spec);
        // Claude ACP reads its profile from cwd; OpenCode reads `OPENCODE_CONFIG`
        // from isolated scratch. Both reach the harness without clobbering files.
        let cfg = HarnessesConfig::load()?;
        let harness_kind = harness::bundle_harness_with(&cfg, bundle)
            .with_context(|| format!("resolving harness for bundle {bundle:?}"))?;
        let scratch = if harness_kind == Harness::ClaudeCode {
            cwd.clone()
        } else {
            crate::config::mosaico_home()
                .join("harness-profiles")
                .join(&spec.slug)
        };
        let mut resolved = harness::resolve_with(&cfg, bundle, spec.profile.as_deref(), &scratch)
            .with_context(|| format!("resolving harness bundle {bundle:?}"))?;
        if let Some(native_agent) = &spec.native_agent {
            harness::apply_native_agent(&mut resolved, native_agent, &scratch)
                .with_context(|| format!("applying native agent {:?}", spec.slug))?;
        }
        if !matches!(resolved.transport, Transport::Acp | Transport::AppServer) {
            anyhow::bail!(
                "harness bundle {bundle:?} is transport {} — not an RPC transport",
                resolved.transport.as_str()
            );
        }
        resolved.profile.materialize()?;
        // The argv actually executed (driver base_argv + profile extra_argv). We
        // record THIS in the session metadata, not the nominal `spec.base_command`
        // that the ACP path never runs (defect #8).
        let argv = resolved.base_argv.clone();
        let callbacks = Callbacks::allow_all(cwd.clone());
        let mut cfg = spawn_config_from_driver(
            resolved.driver,
            &resolved.base_argv,
            &resolved.profile.extra_env,
            cwd,
            callbacks,
        )?;
        crate::session_host::agent_env::assign_launch(&mut cfg.env, &mut cfg.env_remove, spec);
        let dialect = cfg.dialect;
        let (handle, updates) = RpcHandle::spawn(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("spawning RPC harness for bundle {bundle:?}: {e}"))?;
        Ok((handle, dialect, updates, argv))
    }

    fn endpoint_id(slug: &str) -> String {
        // Must be unique across every concurrent session — two same-slug sessions
        // launched in the same wall-clock second would otherwise collide, silently
        // evicting one from the registry and mis-targeting its reaper (defect #1).
        // A process-global monotonic counter makes the id collision-free.
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let now = crate::util::now_secs();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("acp-{slug}-{now}-{}-{seq}", std::process::id())
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
        let (handle, dialect, updates, argv) = Self::spawn_child(spec).await?;
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        let native_id = match dialect {
            Dialect::Acp => {
                let client = AcpClient::new(handle.clone());
                client
                    .initialize()
                    .await
                    .map_err(|e| anyhow::anyhow!("ACP initialize: {e}"))?;
                client
                    .session_new(&cwd)
                    .await
                    .map_err(|e| anyhow::anyhow!("ACP session/new: {e}"))?
            }
            Dialect::AppServer => {
                thread_start_agent::open(&handle, &cwd, spec.native_agent.as_ref()).await?
            }
        };
        let endpoint_id = Self::endpoint_id(&spec.slug);
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

impl SessionTransport for AcpTransport {
    fn kind(&self) -> TransportKind {
        TransportKind::Acp
    }

    async fn launch(&self, spec: &LaunchSpec) -> Result<SessionEndpoint> {
        let open = self.open(spec).await?;
        Ok(SessionEndpoint {
            kind: TransportKind::Acp,
            endpoint_id: open.endpoint_id.clone(),
            watch_pid: open.pid.and_then(|p| i32::try_from(p).ok()),
            meta: Self::synth_meta(spec, &open.endpoint_id, open.pid, &open.argv),
        })
    }

    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint> {
        if resume.native_id.is_empty() {
            anyhow::bail!("session has no resume token (not resumable)");
        }
        let (handle, dialect, updates, argv) = Self::spawn_child(spec).await?;
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        match dialect {
            Dialect::Acp => {
                let client = AcpClient::new(handle.clone());
                client
                    .initialize()
                    .await
                    .map_err(|e| anyhow::anyhow!("ACP initialize (resume): {e}"))?;
                client
                    .session_load(&resume.native_id, &cwd)
                    .await
                    .map_err(|e| anyhow::anyhow!("ACP session/load: {e}"))?;
            }
            Dialect::AppServer => {
                let client = AppServerClient::new(handle.clone());
                client
                    .initialize("mosaico", env!("CARGO_PKG_VERSION"))
                    .await
                    .map_err(|e| anyhow::anyhow!("app-server initialize (resume): {e}"))?;
                client
                    .thread_resume(&resume.native_id, &cwd)
                    .await
                    .map_err(|e| anyhow::anyhow!("app-server thread/resume: {e}"))?;
            }
        }
        let endpoint_id = Self::endpoint_id(&spec.slug);
        let pid = handle.pid;
        register_child(&endpoint_id, handle, resume.native_id.clone(), cwd, updates);
        Ok(SessionEndpoint {
            kind: TransportKind::Acp,
            endpoint_id: endpoint_id.clone(),
            watch_pid: pid.and_then(|p| i32::try_from(p).ok()),
            meta: Self::synth_meta(spec, &endpoint_id, pid, &argv),
        })
    }

    async fn deliver(&self, ep: &EndpointRef, text: &str, submit: bool) -> Result<()> {
        // Snapshot the pieces we need without holding the lock across await.
        let (handle, native_id, dialect, runtime) = {
            let reg = registry().lock().unwrap();
            let child = reg
                .get(&ep.endpoint_id)
                .with_context(|| format!("no ACP session registered for {:?}", ep.endpoint_id))?;
            (
                child.handle.clone(),
                child.native_id.clone(),
                child.handle.dialect,
                child.runtime.clone(),
            )
        };
        // Fire-and-forget (defect #3): injecting a prompt/turn must return
        // promptly like `PtyTransport::deliver`, never block for the whole turn
        // (up to 300s). The turn runs in a detached task; its outcome is logged.
        let text = text.to_string();
        match dialect {
            Dialect::Acp => {
                // ACP has no steer RPC; both submit and non-submit map to a fresh
                // prompt (between-turns delivery).
                if let Ok(mut rt) = runtime.lock() {
                    rt.mark_turn_started();
                }
                spawn_acp_prompt(handle, native_id, text, runtime);
            }
            Dialect::AppServer => {
                // `submit` completes/opens a turn -> always a fresh turn. Only a
                // non-submit ("steer") delivery folds into a running turn.
                let steer = if submit {
                    SteerState::Idle
                } else {
                    runtime
                        .lock()
                        .ok()
                        .map(|rt| rt.steer_state())
                        .unwrap_or(SteerState::Idle)
                };
                match steer {
                    SteerState::Ready(turn_id) => {
                        spawn_app_server_steer(handle, native_id, turn_id, text)
                    }
                    // Defect #2: a turn is running but its id is not known yet.
                    // Starting a fresh turn here would run TWO concurrent turns;
                    // instead gate the steer until the id arrives (bounded wait).
                    SteerState::AwaitingId => {
                        spawn_app_server_steer_pending(handle, native_id, text, runtime)
                    }
                    SteerState::Idle => {
                        if let Ok(mut rt) = runtime.lock() {
                            rt.mark_turn_started();
                        }
                        spawn_app_server_turn(handle, native_id, text, runtime);
                    }
                }
            }
        }
        Ok(())
    }

    fn is_live(&self, ep: &EndpointRef) -> bool {
        registry()
            .lock()
            .unwrap()
            .get(&ep.endpoint_id)
            .map(|c| c.handle.is_alive())
            .unwrap_or(false)
    }

    async fn kill(&self, ep: &EndpointRef) -> Result<()> {
        // Remove eagerly so a concurrent is_live() stops reporting it; the reaper
        // (which may also fire on the resulting exit) tolerates a missing entry.
        let child = registry().lock().unwrap().remove(&ep.endpoint_id);
        if let Some(child) = child {
            let _ = child.cwd; // retained for parity/debugging
            let _ = child.runtime;
            if child.handle.dialect == Dialect::Acp {
                AcpClient::new(child.handle.clone())
                    .session_cancel(&child.native_id)
                    .await;
            }
            child.handle.kill().await;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "acp_reaper_tests.rs"]
mod acp_reaper_tests;
