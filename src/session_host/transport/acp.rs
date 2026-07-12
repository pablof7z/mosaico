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

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result};
use tokio::sync::mpsc;

use super::acp_runtime::{AcpRuntime, SteerState};
use super::{
    EndpointRef, LaunchSpec, ResumeSpec, SessionEndpoint, SessionTransport, TransportKind,
};
use crate::harness::{self, config::HarnessesConfig, Transport};
use crate::rpc_harness::{
    spawn_config_from_driver, AcpClient, AppServerClient, Callbacks, Dialect, RpcHandle,
    SessionUpdate,
};
use crate::session::Harness;

/// A live ACP/app-server child plus its native session token.
struct AcpChild {
    handle: RpcHandle,
    /// ACP `sessionId` or app-server `threadId`.
    native_id: String,
    cwd: std::path::PathBuf,
    /// Captured transcript + running-turn state, fed by the update-drain task.
    runtime: Arc<Mutex<AcpRuntime>>,
}

fn registry() -> &'static Mutex<HashMap<String, AcpChild>> {
    static REG: OnceLock<Mutex<HashMap<String, AcpChild>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct AcpTransport;

/// The outcome of opening (or resuming) an RPC-hosted session, before it is
/// wrapped into a `SessionEndpoint`.
pub struct AcpOpen {
    pub endpoint_id: String,
    pub native_id: String,
    pub pid: Option<u32>,
}

impl AcpTransport {
    /// Resolve the harness bundle for `slug` and spawn its RPC child, returning
    /// the live handle + dialect + the update stream. Shared by launch/resume.
    async fn spawn_child(
        spec: &LaunchSpec,
    ) -> Result<(RpcHandle, Dialect, mpsc::Receiver<SessionUpdate>)> {
        let cwd = std::path::PathBuf::from(&spec.abs_path);
        // Profile placement (defect #5): a CwdSettingsFile profile must land where
        // the harness actually reads it. The claude-code-acp adapter reads
        // `<cwd>/.claude/settings.json` and ignores `--settings`, so its profile
        // scratch dir MUST be the session cwd — an out-of-tree scratch would never
        // be consulted. opencode instead reads `OPENCODE_CONFIG` (pointed at an
        // out-of-tree scratch), so it stays there and never clobbers a repo file.
        let cfg = HarnessesConfig::load()?;
        let harness_kind = harness::bundle_harness_with(&cfg, &spec.slug)
            .with_context(|| format!("resolving harness for bundle {:?}", spec.slug))?;
        let scratch = if harness_kind == Harness::ClaudeCode {
            cwd.clone()
        } else {
            crate::config::edge_home()
                .join("harness-profiles")
                .join(&spec.slug)
        };
        let resolved = harness::resolve_with(&cfg, &spec.slug, &scratch)
            .with_context(|| format!("resolving harness bundle {:?}", spec.slug))?;
        if !matches!(resolved.transport, Transport::Acp | Transport::AppServer) {
            anyhow::bail!(
                "harness bundle {:?} is transport {} — not an RPC transport",
                spec.slug,
                resolved.transport.as_str()
            );
        }
        // Materialize any profile settings files before launch.
        for (path, contents) in &resolved.profile.files {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating profile dir {}", parent.display()))?;
            }
            std::fs::write(path, contents)
                .with_context(|| format!("writing profile file {}", path.display()))?;
        }
        let callbacks = Callbacks::allow_all(cwd.clone());
        let cfg = spawn_config_from_driver(
            resolved.driver,
            &resolved.base_argv,
            &resolved.profile.extra_env,
            cwd,
            callbacks,
        )?;
        let dialect = cfg.dialect;
        let (handle, updates) = RpcHandle::spawn(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("spawning RPC harness for {:?}: {e}", spec.slug))?;
        Ok((handle, dialect, updates))
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
        format!("te-acp-{slug}-{now}-{}-{seq}", std::process::id())
    }

    fn synth_meta(
        spec: &LaunchSpec,
        endpoint_id: &str,
        pid: Option<u32>,
    ) -> crate::pty::LaunchMetadata {
        crate::pty::LaunchMetadata {
            id: endpoint_id.to_string(),
            socket: String::new(),
            // ACP has no PTY supervisor; the child *is* the process the daemon
            // must watch for liveness, so carry its pid here (bootstrap turns
            // `supervisor_pid` into the session's `watch_pid`).
            supervisor_pid: pid.unwrap_or(0),
            agent: spec.slug.clone(),
            root: spec.root.clone(),
            cwd: spec.abs_path.clone(),
            ephemeral: spec.ephemeral,
            command: spec.base_command.clone(),
        }
    }

    /// Register a freshly-opened child: store it, drain its update stream into a
    /// shared [`AcpRuntime`], and start the reaper that reclaims it on exit.
    fn register_child(
        endpoint_id: &str,
        handle: RpcHandle,
        native_id: String,
        cwd: std::path::PathBuf,
        mut updates: mpsc::Receiver<SessionUpdate>,
    ) {
        let runtime = Arc::new(Mutex::new(AcpRuntime::default()));
        // Update-drain task (defect #6): keep the receiver alive and fold each
        // notification into the runtime so RPC sessions capture a transcript and
        // track the live turn id.
        let rt_updates = runtime.clone();
        tokio::spawn(async move {
            while let Some(u) = updates.recv().await {
                if let Ok(mut rt) = rt_updates.lock() {
                    rt.note_update(&u.method, &u.params);
                }
            }
        });
        // Reaper task (defect #1): await child exit, drop the registry entry, and
        // `wait()` the zombie.
        let reaper_handle = handle.clone();
        let reaper_id = endpoint_id.to_string();
        tokio::spawn(async move {
            reaper_handle.wait_exit().await;
            registry().lock().unwrap().remove(&reaper_id);
            tracing::debug!(endpoint = %reaper_id, "acp child exited; registry entry reaped");
        });
        registry().lock().unwrap().insert(
            endpoint_id.to_string(),
            AcpChild {
                handle,
                native_id,
                cwd,
                runtime,
            },
        );
    }

    /// Open a fresh session and register it; returns the open descriptor.
    pub async fn open(&self, spec: &LaunchSpec) -> Result<AcpOpen> {
        let (handle, dialect, updates) = Self::spawn_child(spec).await?;
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
                let client = AppServerClient::new(handle.clone());
                client
                    .initialize("tenex-edge", env!("CARGO_PKG_VERSION"))
                    .await
                    .map_err(|e| anyhow::anyhow!("app-server initialize: {e}"))?;
                client
                    .thread_start(&cwd)
                    .await
                    .map_err(|e| anyhow::anyhow!("app-server thread/start: {e}"))?
            }
        };
        let endpoint_id = Self::endpoint_id(&spec.slug);
        let pid = handle.pid;
        Self::register_child(&endpoint_id, handle, native_id.clone(), cwd, updates);
        Ok(AcpOpen {
            endpoint_id,
            native_id,
            pid,
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
            meta: Self::synth_meta(spec, &open.endpoint_id, open.pid),
        })
    }

    async fn resume(&self, spec: &LaunchSpec, resume: &ResumeSpec) -> Result<SessionEndpoint> {
        if resume.native_id.is_empty() {
            anyhow::bail!("session has no resume token (not resumable)");
        }
        let (handle, dialect, updates) = Self::spawn_child(spec).await?;
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
                    .initialize("tenex-edge", env!("CARGO_PKG_VERSION"))
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
        Self::register_child(&endpoint_id, handle, resume.native_id.clone(), cwd, updates);
        Ok(SessionEndpoint {
            kind: TransportKind::Acp,
            endpoint_id: endpoint_id.clone(),
            watch_pid: pid.and_then(|p| i32::try_from(p).ok()),
            meta: Self::synth_meta(spec, &endpoint_id, pid),
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

/// A snapshot of the captured assistant transcript for an ACP endpoint, if it is
/// still registered. Used by the status distiller for RPC-hosted sessions.
pub fn transcript_snapshot(endpoint_id: &str) -> Option<String> {
    let reg = registry().lock().unwrap();
    let child = reg.get(endpoint_id)?;
    child.runtime.lock().ok().map(|rt| rt.transcript())
}

fn spawn_acp_prompt(
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

fn spawn_app_server_turn(
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

fn spawn_app_server_steer(handle: RpcHandle, native_id: String, turn_id: String, text: String) {
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
fn spawn_app_server_steer_pending(
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

#[cfg(test)]
#[path = "acp_reaper_tests.rs"]
mod acp_reaper_tests;
