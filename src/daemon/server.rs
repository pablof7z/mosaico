//! The daemon process: sole owner of state.db AND the single relay connection.
//!
//! Started as the hidden `tenex-edge __daemon` subcommand by a thin client's
//! spawn-if-absent path. See docs/daemon-design.md. Responsibilities:
//!   - bind the UDS under the startup `flock`, reclaiming a stale socket;
//!   - own one `Store` (single SQLite writer) and one `Transport` (one relay
//!     connection) with a single union subscription across all hosted agents;
//!   - run per-session engine tasks (the relocated `run_session_in_daemon`);
//!   - demux incoming relay events once and route mentions to the right agent's
//!     inbox (multi-agent aware); prune stale peers; serve RPCs; idle-exit.

use super::client::StartupLock;
use super::protocol::{
    protocol_version, Hello, PleaseExit, Request, Response, Welcome, ERR_PROTOCOL_SKEW,
};
use super::{lock_path, socket_path, store_path};
use crate::codec::{Codec, Kind1Codec, SubScope};
use crate::config::{self, Config};
use crate::domain::{DomainEvent, Mention};
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, route_mention_into, route_mention_into_with_id, EngineParams};
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, session_short_code, SessionId};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

mod admin;
mod awareness;
mod connection;
mod demux;
mod inbox;
mod lifecycle;
mod messaging;
mod session;

const PRUNE_PEER_AFTER_SECS: u64 = 600;

fn grace() -> Duration {
    Duration::from_secs(env_u64("TENEX_EDGE_DAEMON_GRACE_S", 120))
}

#[derive(Clone)]
struct HostedAgent {
    keys: Keys,
}

struct SessionHandle {
    cancel: Arc<Notify>,
}

/// Shared daemon state. The `Store` is behind an `Arc<Mutex<…>>` shared with
/// session tasks; the guard is held only across synchronous rusqlite calls,
/// NEVER across `.await`. One process + one connection = the single writer.
pub struct DaemonState {
    store: Arc<Mutex<Store>>,
    transport: Arc<Transport>,
    codec: Kind1Codec,
    cfg: Config,
    host: String,
    owners: Vec<String>,
    /// Hosted local agent pubkeys (the "me set" for self-skip + routing).
    hosted: Mutex<HashMap<String, HostedAgent>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    subscribed_projects: Mutex<Vec<String>>,
    mention_notify: Notify,
    /// Broadcast of decoded fabric events to live `tail` clients.
    tail_tx: tokio::sync::broadcast::Sender<DomainEvent>,
    open_clients: Mutex<u64>,
    liveness_changed: Notify,
    shutdown: Notify,
    /// Ring buffer of recently-processed event IDs used to deduplicate relay
    /// deliveries when multiple subscriptions match the same event.
    seen_events: Mutex<VecDeque<String>>,
}

impl DaemonState {
    fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
    fn hosted_pubkeys(&self) -> Vec<String> {
        self.hosted.lock().unwrap().keys().cloned().collect()
    }
    fn keys_for(&self, pubkey: &str) -> Option<Keys> {
        self.hosted
            .lock()
            .unwrap()
            .get(pubkey)
            .map(|h| h.keys.clone())
    }
    fn live_session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

// ── entry point ──────────────────────────────────────────────────────────────

pub async fn run() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;

    let lock = match StartupLock::try_acquire()? {
        Some(l) => l,
        None => {
            eprintln!("[daemon] another daemon already running; exiting");
            return Ok(());
        }
    };
    let listener = bind_socket()?;
    eprintln!("[daemon] listening on {}", socket_path().display());

    let cfg = Config::load().context("loading config")?;
    let host = cfg.host.clone();
    let owners = cfg.whitelisted_pubkeys.clone();

    // One relay connection. AUTH identity is irrelevant to delivery (verified:
    // an A-authed connection receives events p-tagged to B), so authenticate
    // with a stable daemon key and sign each event with its true author.
    let auth_keys = identity::load_or_create(&config::edge_home(), "tenex-edge-daemon", now_secs())
        .map(|i| i.keys)
        .unwrap_or_else(|_| Keys::generate());
    let transport = Arc::new(
        Transport::connect(&cfg.relays, auth_keys)
            .await
            .context("daemon relay connect")?,
    );

    let state = Arc::new(DaemonState {
        store: Arc::new(Mutex::new(Store::open(&store_path())?)),
        transport,
        codec: Kind1Codec,
        cfg,
        host,
        owners,
        hosted: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        subscribed_projects: Mutex::new(Vec::new()),
        mention_notify: Notify::new(),
        tail_tx: tokio::sync::broadcast::channel(256).0,
        open_clients: Mutex::new(0),
        liveness_changed: Notify::new(),
        shutdown: Notify::new(),
        seen_events: Mutex::new(VecDeque::with_capacity(512)),
    });

    lifecycle::reconcile_sessions(&state).await;
    demux::spawn_demux(state.clone());
    demux::spawn_pruner(state.clone());
    demux::spawn_idle_watcher(state.clone());

    let accept_state = state.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let st = accept_state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = connection::serve_connection(st, stream).await {
                            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                                eprintln!("[daemon] connection error: {e:#}");
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[daemon] accept error: {e}");
                    break;
                }
            }
        }
    });

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    tokio::select! {
        _ = state.shutdown.notified() => {}
        _ = async { match &mut sigterm { Some(s) => { s.recv().await; }, None => std::future::pending().await } } => {}
    }
    eprintln!("[daemon] shutting down");
    accept.abort();
    cleanup();
    state.transport.shutdown().await;
    drop(lock);
    Ok(())
}

fn bind_socket() -> Result<UnixListener> {
    let sock = socket_path();
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    UnixListener::bind(&sock).with_context(|| format!("binding {}", sock.display()))
}

fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
    let _ = std::fs::remove_file(lock_path());
}

// ── small helpers ─────────────────────────────────────────────────────────────

impl DaemonState {
    fn tail_subscribe(&self) -> tokio::sync::broadcast::Receiver<DomainEvent> {
        self.tail_tx.subscribe()
    }
    fn tail_tx_send(
        &self,
        de: DomainEvent,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<DomainEvent>> {
        self.tail_tx.send(de)
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
