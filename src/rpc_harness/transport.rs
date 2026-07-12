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
    Arc<Mutex<HashMap<i64, oneshot::Sender<Result<serde_json::Value, RpcErrorObject>>>>>;
pub(super) type TurnWaiters = Arc<Mutex<HashMap<String, oneshot::Sender<serde_json::Value>>>>;

/// Cheaply-cloneable handle to a live harness child.
#[derive(Clone)]
pub struct RpcHandle {
    ids: Arc<AtomicI64>,
    writer: mpsc::Sender<String>,
    pending: PendingMap,
    turn_waiters: TurnWaiters,
    alive: Arc<AtomicBool>,
    child: Arc<tokio::sync::Mutex<Child>>,
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
    ) -> Result<(RpcHandle, mpsc::Receiver<SessionUpdate>), RpcError> {
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

        let mut child = cmd.spawn().map_err(RpcError::Transport)?;
        let pid = child.id();
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");

        let (write_tx, write_rx) = mpsc::channel::<String>(256);
        let (update_tx, update_rx) = mpsc::channel::<SessionUpdate>(256);
        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let turn_waiters: TurnWaiters = Arc::new(Mutex::new(HashMap::new()));
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
            turn_waiters.clone(),
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
                turn_waiters,
                alive,
                child: Arc::new(tokio::sync::Mutex::new(child)),
                exit: exit_rx,
                dialect: cfg.dialect,
                pid,
            },
            update_rx,
        ))
    }

    fn next_id(&self) -> i64 {
        self.ids.fetch_add(1, Ordering::Relaxed)
    }

    /// Send a request and await its correlated response.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RpcError> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        // Insert the waiter and observe `alive` under the *same* lock the reader
        // takes to drain on EOF (it sets `alive=false` before acquiring the
        // pending lock). This closes the orphaned-pending race: if we see the
        // child alive here, the drain has not run yet and is guaranteed to fail
        // this entry; if it already ran, we bail fast with `ChildExited` rather
        // than awaiting a oneshot nobody will ever resolve.
        {
            let mut pending = self.pending.lock().unwrap();
            if !self.alive.load(Ordering::Relaxed) {
                return Err(RpcError::ChildExited);
            }
            pending.insert(id, tx);
        }
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        })
        .to_string();
        if self.writer.send(line).await.is_err() {
            self.pending.lock().unwrap().remove(&id);
            return Err(RpcError::ChildExited);
        }
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(RpcError::Protocol(e)),
            Err(_) => Err(RpcError::ChildExited),
        }
    }

    /// Send a request with a timeout.
    pub async fn request_timeout(
        &self,
        method: &str,
        params: serde_json::Value,
        dur: std::time::Duration,
    ) -> Result<serde_json::Value, RpcError> {
        match tokio::time::timeout(dur, self.request(method, params)).await {
            Ok(r) => r,
            Err(_) => Err(RpcError::Timeout),
        }
    }

    /// Fire-and-forget notification (no id, no await).
    pub async fn notify(&self, method: &str, params: serde_json::Value) {
        let line = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string();
        let _ = self.writer.send(line).await;
    }

    /// Register a turn-completion waiter (app-server: `turn/completed` arrives
    /// as a notification, not the `turn/start` response). Returns a receiver the
    /// caller awaits after sending `turn/start`.
    pub fn register_turn_waiter(&self, key: &str) -> oneshot::Receiver<serde_json::Value> {
        let (tx, rx) = oneshot::channel();
        self.turn_waiters
            .lock()
            .unwrap()
            .insert(key.to_string(), tx);
        rx
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

    /// Kill the child process. The reader task observes the resulting stdout EOF
    /// and fires the exit signal, which the reaper turns into a `wait()`.
    pub async fn kill(&self) {
        self.alive.store(false, Ordering::Relaxed);
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
    }
}
