//! `NdjsonTransport`: owns a child harness process and speaks newline-delimited
//! JSON-RPC over its stdio.
//!
//! Async shape: one reader task owns child stdout + the correlation map; one
//! writer task drains a `mpsc<String>` to child stdin; a third task drains
//! stderr to `tracing`. Public request methods = "allocate id, push line, await
//! oneshot". This is the deadlock-free pattern required because the agent
//! interleaves its own requests + id-less notifications between our request and
//! its response.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, watch};

use super::callbacks::Callbacks;
use super::io_tasks::{reader_task, stderr_task, writer_task};
use super::protocol::{Dialect, RpcErrorObject, SessionUpdate};

#[path = "transport/turn_routing.rs"]
mod turn_routing;
pub(super) use turn_routing::{AppServerRouting, TurnObserver, TurnSignal};

#[path = "transport/request.rs"]
mod request;

#[cfg(unix)]
#[path = "transport/process_group.rs"]
mod process_group;

/// Error surface for RPC calls.
#[derive(Debug)]
pub enum RpcError {
    Transport(std::io::Error),
    Protocol(RpcErrorObject),
    ChildExited,
    Decode(serde_json::Error),
    Timeout,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Transport(e) => write!(f, "transport: {e}"),
            RpcError::Protocol(e) => write!(f, "rpc error {}: {}", e.code, e.message),
            RpcError::ChildExited => write!(f, "child harness exited before response"),
            RpcError::Decode(e) => write!(f, "decode: {e}"),
            RpcError::Timeout => write!(f, "rpc timed out"),
        }
    }
}

impl std::error::Error for RpcError {}

/// Config for spawning + framing a harness child.
pub struct SpawnConfig {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub dialect: Dialect,
    pub callbacks: Callbacks,
}

pub(super) type PendingMap =
    Arc<Mutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, RpcError>>>>>;

/// Cheaply-cloneable handle to a live harness child.
#[derive(Clone)]
pub struct RpcHandle {
    ids: Arc<AtomicI64>,
    writer: mpsc::Sender<String>,
    pending: PendingMap,
    app_server_routing: AppServerRouting,
    alive: Arc<AtomicBool>,
    child: Arc<tokio::sync::Mutex<Child>>,
    #[cfg(unix)]
    process_group: Option<i32>,
    /// Flips to `true` exactly once when the child's stdout closes (reader EOF)
    /// or `kill()` is called. A reaper task awaits this to remove the child from
    /// its registry and `wait()` the zombie — no per-child polling.
    exit: watch::Receiver<bool>,
    pub dialect: Dialect,
    pub pid: Option<u32>,
}

impl RpcHandle {
    /// Spawn a harness child and start its reader/writer tasks. Returns the
    /// handle plus a receiver of high-level `session/update` (and other)
    /// notifications.
    pub async fn spawn(
        cfg: SpawnConfig,
    ) -> Result<(RpcHandle, mpsc::UnboundedReceiver<SessionUpdate>), RpcError> {
        let mut cmd = Command::new(&cfg.program);
        cmd.args(&cfg.args)
            .current_dir(&cfg.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for k in &cfg.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &cfg.env {
            cmd.env(k, v);
        }
        #[cfg(unix)]
        process_group::isolate(&mut cmd);

        let mut child = cmd.spawn().map_err(RpcError::Transport)?;
        let pid = child.id();
        #[cfg(unix)]
        let process_group = pid.and_then(|pid| i32::try_from(pid).ok());
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (write_tx, write_rx) = mpsc::channel::<String>(256);
        // Updates use an UNBOUNDED channel (defect #15): a bounded `try_send` drops
        // `session/update` notifications — including `turn/started`, which carries
        // the turn_id that steer depends on — if the channel is momentarily full or
        // the drain task hasn't registered yet. The drain always consumes promptly,
        // so unbounded growth is bounded by one turn's notifications in practice,
        // and we never silently lose a turn id.
        let (update_tx, update_rx) = mpsc::unbounded_channel::<SessionUpdate>();
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let app_server_routing = AppServerRouting::default();
        let alive = Arc::new(AtomicBool::new(true));
        let (exit_tx, exit_rx) = watch::channel(false);

        // Writer task.
        tokio::spawn(writer_task(stdin, write_rx));
        // Stderr drain.
        tokio::spawn(stderr_task(stderr, cfg.program.clone()));
        // Reader task.
        tokio::spawn(reader_task(
            stdout,
            pending.clone(),
            app_server_routing.clone(),
            write_tx.clone(),
            update_tx,
            cfg.callbacks,
            alive.clone(),
            exit_tx,
        ));

        Ok((
            RpcHandle {
                ids: Arc::new(AtomicI64::new(1)),
                writer: write_tx,
                pending,
                app_server_routing,
                alive,
                child: Arc::new(tokio::sync::Mutex::new(child)),
                #[cfg(unix)]
                process_group,
                exit: exit_rx,
                dialect: cfg.dialect,
                pid,
            },
            update_rx,
        ))
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    /// Await the child's exit (stdout EOF or `kill()`), then `wait()` it so the
    /// OS releases the zombie. Returns promptly if the child has already exited.
    /// A single reaper task per child owns this; it never blocks `kill()` because
    /// it only takes the child lock *after* the exit signal fires, by which point
    /// the process is already terminating.
    pub async fn wait_exit(&self) {
        let mut rx = self.exit.clone();
        // If not yet dead, wait for the flip; ignore a closed sender (means the
        // reader task dropped after signalling).
        if !*rx.borrow() {
            let _ = rx.wait_for(|dead| *dead).await;
        }
        let mut child = self.child.lock().await;
        let _ = child.wait().await;
    }

    /// Kill and reap the child process. Returning success is an ownership
    /// boundary: callers may remove endpoint registry and durable runtime state
    /// only after this method has observed process exit.
    pub async fn kill(&self) -> std::io::Result<()> {
        let mut child = self.child.lock().await;
        let running = child.try_wait()?.is_none();
        #[cfg(unix)]
        let killed_group = if let Some(group) = self.process_group {
            process_group::signal(group)?
        } else {
            false
        };
        #[cfg(not(unix))]
        let killed_group = false;
        if running {
            if !killed_group {
                child.start_kill()?;
            }
            tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
                .await
                .map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "timed out waiting for RPC harness child to exit after kill",
                    )
                })??;
        }
        drop(child);
        #[cfg(unix)]
        if let Some(group) = self.process_group {
            process_group::wait_exit(group).await?;
        }
        self.alive.store(false, Ordering::Relaxed);
        Ok(())
    }
}
