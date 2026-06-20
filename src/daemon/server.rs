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
use super::tail_event::TailEvent;
use super::{lock_path, socket_path, store_path};
use crate::config::{self, Config};
use crate::domain::{ChatMessage, DomainEvent, Mention};
use crate::fabric::provider::Kind1Nip29Provider;
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, route_mention_into_with_id, EngineParams};
use crate::session::{derive_status, Harness, SessionObservation, SessionSnapshot};
use crate::state::{ChatInboxRow, ChatLogRow, InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, pubkey_short, session_codename, SessionId};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

mod tmux_rpc;

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

/// Metadata tracked per live peer session for join/leave derivation.
#[derive(Clone)]
struct PeerTracked {
    first_seen: u64,
    project: String,
    slug: String,
    host: String,
}

/// Shared daemon state. The `Store` is behind an `Arc<Mutex<…>>` shared with
/// session tasks; the guard is held only across synchronous rusqlite calls,
/// NEVER across `.await`. One process + one connection = the single writer.
pub struct DaemonState {
    store: Arc<Mutex<Store>>,
    transport: Arc<Transport>,
    provider: Arc<Kind1Nip29Provider>,
    cfg: Config,
    host: String,
    owners: Vec<String>,
    /// Hosted local agent pubkeys (the "me set" for self-skip + routing).
    hosted: Mutex<HashMap<String, HostedAgent>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    subscribed_projects: Mutex<Vec<String>>,
    /// Structured tail event broadcast replacing the old DomainEvent bus.
    tail_tx: tokio::sync::broadcast::Sender<TailEvent>,
    open_clients: Mutex<u64>,
    liveness_changed: Notify,
    shutdown: Notify,
    /// In-memory peer-session tracking for join/leave derivation.
    /// key = session_id. Populated on first-seen presence; cleared on leave.
    peer_sessions: Mutex<HashMap<String, PeerTracked>>,
    /// Bounded first-sight tracking of native event ids: the relay pool
    /// notifies once per matching subscription, so the same event arrives many
    /// times. Set + insertion-order queue, capped at SEEN_EVENTS_CAP.
    seen_events: Mutex<(
        std::collections::HashSet<String>,
        std::collections::VecDeque<String>,
    )>,
    /// Pubkeys for which a Profile event has already been emitted, for first-seen dedup.
    seen_profiles: Mutex<std::collections::HashSet<String>>,
    /// Last-seen (title, active) keyed by the SESSION id (canonical for locals,
    /// native for peers) for tail dedup. Keying by session — not (pubkey,project)
    /// — is the multi-session fix: sibling sessions of one agent each emit their
    /// own status transitions. Tracking `active` too means an active→idle flip
    /// emits a tail event even though the persistent title text is unchanged.
    last_status: Mutex<HashMap<String, (String, bool)>>,
    /// Wakes the status-outbox drainer the instant a transition enqueues a publish.
    status_outbox_notify: Notify,
}

impl DaemonState {
    pub(crate) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
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
    // Include the indexer relay in the transport pool so kind:0 publishes reach
    // it and kind:0 subscriptions also query it for profile discovery. Deduped
    // in case someone lists purplepag.es in their main relays too.
    let transport_relays: Vec<String> = {
        let mut v = cfg.relays.clone();
        if !v.iter().any(|r| r == &cfg.indexer_relay) {
            v.push(cfg.indexer_relay.clone());
        }
        v
    };
    let transport = Arc::new(
        Transport::connect(&transport_relays, auth_keys)
            .await
            .context("daemon relay connect")?,
    );

    let store = Arc::new(Mutex::new(Store::open(&store_path())?));
    let provider = Arc::new(Kind1Nip29Provider::new(
        transport.clone(),
        store.clone(),
        cfg.user_nsec.clone(),
        cfg.whitelisted_pubkeys.clone(),
        &cfg.relays, // provider_instance hashes main relays only, not indexer
    ));
    let state = Arc::new(DaemonState {
        store,
        transport,
        provider,
        cfg,
        host,
        owners,
        hosted: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        subscribed_projects: Mutex::new(Vec::new()),
        tail_tx: tokio::sync::broadcast::channel(512).0,
        open_clients: Mutex::new(0),
        liveness_changed: Notify::new(),
        shutdown: Notify::new(),
        peer_sessions: Mutex::new(HashMap::new()),
        seen_events: Mutex::new((
            std::collections::HashSet::new(),
            std::collections::VecDeque::new(),
        )),
        seen_profiles: Mutex::new(std::collections::HashSet::new()),
        last_status: Mutex::new(HashMap::new()),
        status_outbox_notify: Notify::new(),
    });

    // Idempotent read-model backfill: populate canonical `projects` + `membership`
    // tables from legacy data so readers have a consistent origin on every start.
    // Best-effort: a backfill error must not prevent startup.
    {
        let pi = state.provider.provider_instance.clone();
        state.with_store(|s| {
            s.backfill_kind1_nip29_origins(&pi, now_secs()).ok();
        });
    }

    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    spawn_idle_watcher(state.clone());
    spawn_status_outbox_drainer(state.clone());
    spawn_status_heartbeat_publisher(state.clone());

    let accept_state = state.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let st = accept_state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_connection(st, stream).await {
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

    let reconcile_state = state.clone();
    tokio::spawn(async move {
        reconcile_sessions(&reconcile_state).await;
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

// ── connection handling ──────────────────────────────────────────────────────

async fn serve_connection(state: Arc<DaemonState>, stream: UnixStream) -> Result<()> {
    let (rh, wh) = stream.into_split();
    let mut reader = BufReader::new(rh);
    let mut writer = wh;

    let mut first = String::new();
    if reader.read_line(&mut first).await? == 0 {
        return Ok(());
    }
    let hello: Hello = serde_json::from_str(first.trim_end()).context("parsing hello")?;
    write_json(
        &mut writer,
        &Welcome {
            protocol: protocol_version(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .await?;

    if hello.protocol > protocol_version() {
        let mut line = String::new();
        if reader.read_line(&mut line).await? > 0
            && serde_json::from_str::<PleaseExit>(line.trim_end()).is_ok()
        {
            eprintln!(
                "[daemon] newer client (protocol {}); exiting for re-exec",
                hello.protocol
            );
            state.shutdown.notify_waiters();
        }
        let _ = write_json(
            &mut writer,
            &Response::err(0, ERR_PROTOCOL_SKEW, "daemon exiting for re-exec"),
        )
        .await;
        return Ok(());
    }

    {
        *state.open_clients.lock().unwrap() += 1;
        state.liveness_changed.notify_waiters();
    }
    let _guard = ClientGuard(state.clone());

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                write_json(&mut writer, &Response::err(0, "bad_request", e.to_string())).await?;
                continue;
            }
        };
        match req.method.as_str() {
            "tail" => {
                handle_tail(&state, req.id, &req.params, &mut writer).await?;
                break; // tail owns the connection until the client disconnects
            }
            "chat_read" => {
                handle_chat_read(&state, req.id, &req.params, &mut writer).await?;
                break; // chat_read may own the connection for --live
            }
            "session_start" => {
                handle_session_start(&state, req.id, &req.params, &mut writer).await?;
            }
            _ => {
                let resp = dispatch(&state, &req).await;
                write_json(&mut writer, &resp).await?;
            }
        }
    }
    Ok(())
}

struct ClientGuard(Arc<DaemonState>);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut n = self.0.open_clients.lock().unwrap();
        *n = n.saturating_sub(1);
        self.0.liveness_changed.notify_waiters();
    }
}

async fn write_json<T: serde::Serialize, W: AsyncWriteExt + Unpin>(w: &mut W, v: &T) -> Result<()> {
    let mut line = serde_json::to_string(v)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

#[derive(Clone)]
struct InitProgress {
    started: Instant,
    tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
}

impl InitProgress {
    fn new(tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>) -> Self {
        Self {
            started: Instant::now(),
            tx,
        }
    }

    fn emit(&self, phase: &str, message: impl Into<String>) {
        let _ = self.tx.send(serde_json::json!({
            "kind": "init_progress",
            "phase": phase,
            "message": message.into(),
            "elapsed_ms": self.started.elapsed().as_millis() as u64,
        }));
    }
}

async fn handle_session_start<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = InitProgress::new(tx);
    let fut = rpc_session_start(state, params, Some(progress));
    tokio::pin!(fut);

    let result = loop {
        tokio::select! {
            Some(item) = rx.recv() => {
                write_json(writer, &Response::item(id, item)).await?;
            }
            result = &mut fut => break result,
        }
    };

    while let Ok(item) = rx.try_recv() {
        write_json(writer, &Response::item(id, item)).await?;
    }

    let resp = match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, "rpc_error", format!("{e:#}")),
    };
    write_json(writer, &resp).await
}

// ── dispatch (one-shot verbs) ────────────────────────────────────────────────

async fn dispatch(state: &Arc<DaemonState>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "ping" => Ok(serde_json::json!({"pong": true})),
        "who" => rpc_who(state, &req.params),
        "session_start" => rpc_session_start(state, &req.params, None).await,
        "session_end" => rpc_session_end(state, &req.params),
        "send_message" => rpc_send_message(state, &req.params).await,
        "chat_write" => rpc_chat_write(state, &req.params).await,
        "propose" => rpc_propose(state, &req.params).await,
        "inbox" => rpc_inbox(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "user_prompt" => rpc_user_prompt(state, &req.params).await,
        "project_list" => rpc_project_list(state).await,
        "project_edit" => rpc_project_edit(state, &req.params).await,
        "project_members" => rpc_project_members(state, &req.params).await,
        "project_add" => rpc_project_add(state, &req.params).await,
        "project_remove" => rpc_project_remove(state, &req.params).await,
        "publish_profile" => rpc_publish_profile(state, &req.params).await,
        "inbox_reply" => rpc_inbox_reply(state, &req.params).await,
        "statusline" => rpc_statusline(state, &req.params),
        "whoami" => rpc_whoami(state, &req.params),
        "list_threads" => rpc_list_threads(state, &req.params).await,
        "messages" => rpc_messages(state, &req.params),
        "thread_meta" => rpc_thread_meta(state, &req.params),
        "tmux_status" => tmux_rpc::rpc_tmux_status(state),
        "tmux_send" => tmux_rpc::rpc_tmux_send(state, &req.params).await,
        "tmux_spawn" => tmux_rpc::rpc_tmux_spawn(state, &req.params).await,
        "tmux_attach" => tmux_rpc::rpc_tmux_attach(state, &req.params),
        "tmux_resume" => tmux_rpc::rpc_tmux_resume(state, &req.params).await,
        "tmux_resumable" => tmux_rpc::rpc_tmux_resumable(state),
        other => Err(anyhow::anyhow!("unknown method {other}")),
    };
    match result {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, "rpc_error", format!("{e:#}")),
    }
}

// ── session resolution (daemon-side, identical to the old CLI) ───────────────

/// Resolve the caller's session like the pre-daemon CLI did, but AGENT-SCOPED:
/// explicit id → the `env_session` the host exported → most-recent alive session
/// for the project of `cwd` **belonging to the invoking agent** (`agent`, from
/// `$TENEX_EDGE_AGENT`). The agent-scoped fallback is the fix for the bug where a
/// `claude` send-message was signed/recorded as `opencode` merely because an
/// opencode session was the latest-active in the project. If `agent` is unknown
/// (older clients that don't thread it), fall back to the agent-agnostic
/// latest-alive lookup to preserve prior behavior.
fn resolve_session(
    state: &Arc<DaemonState>,
    explicit: Option<&str>,
    env_session: Option<&str>,
    cwd: Option<&str>,
    agent: Option<&str>,
) -> Result<crate::state::SessionRecord> {
    resolve_session_inner(state, explicit, env_session, cwd, agent, true)
}

/// Resolve the caller's session. `allow_project_fallback` controls the LAST
/// resort: when the caller carries no session/agent signal at all, `true` picks
/// the project's latest-alive session (fine for host-facing commands run from a
/// repo), while `false` errors instead — used by `whoami`, which is only
/// meaningful when actually run *as* an agent and must not silently bind to some
/// arbitrary sibling session when run from a bare terminal.
fn resolve_session_inner(
    state: &Arc<DaemonState>,
    explicit: Option<&str>,
    env_session: Option<&str>,
    cwd: Option<&str>,
    agent: Option<&str>,
    allow_project_fallback: bool,
) -> Result<crate::state::SessionRecord> {
    if let Some(id) = explicit.filter(|s| !s.is_empty()) {
        return state
            .with_store(|s| s.get_session(id))
            .with_context(|| format!("unknown session {id}"))?
            .with_context(|| format!("unknown session {id}"));
    }
    if let Some(id) = env_session.filter(|s| !s.is_empty()) {
        if let Some(rec) = state.with_store(|s| s.get_session(id)).ok().flatten() {
            return Ok(rec);
        }
    }
    let cwd = cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = crate::project::resolve(&cwd);
    if let Some(agent) = agent.filter(|a| !a.is_empty()) {
        if let Some(rec) =
            state.with_store(|s| s.latest_alive_session_for_agent_in_project(agent, &project))?
        {
            return Ok(rec);
        }
        anyhow::bail!(
            "no active tenex-edge session for agent {agent:?} in project {project:?} (run session-start, or pass --session)"
        );
    }
    if !allow_project_fallback {
        anyhow::bail!(
            "not running as a tenex-edge agent: no --session, TENEX_EDGE_SESSION, or TENEX_EDGE_AGENT in scope"
        );
    }
    state
        .with_store(|s| s.latest_alive_session_for_project(&project))?
        .with_context(|| {
            format!("no active tenex-edge session for project {project:?} (run session-start, or pass --session)")
        })
}

// ── who ──────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct WhoParams {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    all: bool,
    #[serde(default)]
    all_projects: bool,
    #[serde(default)]
    cwd: Option<String>,
}

/// `who`: build the snapshot with the SAME function the CLI used. The client
/// renders it with the existing renderers, so output is byte-identical. The
/// daemon resolves the current project the same way the old CLI did.
fn rpc_who(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: WhoParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let current_project = if p.all_projects {
        None
    } else {
        Some(p.project.clone().unwrap_or_else(|| {
            let cwd = p
                .cwd
                .clone()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            crate::project::resolve(&cwd)
        }))
    };
    let now = now_secs();
    let host = state.host.clone();
    let snapshot = state.with_store(|s| {
        crate::cli::load_who_snapshot(s, current_project.as_deref(), p.all, now, &host)
    })?;
    Ok(serde_json::to_value(snapshot)?)
}

// ── session_start / session_end ──────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SessionStartParams {
    agent: String,
    /// The harness-native external session id. Hooks send it as
    /// `harness_session_id`; the legacy/CLI path sends `session_id`. Either is
    /// accepted — it is ONLY a locator for `session_aliases`, never the identity.
    #[serde(default, alias = "harness_session_id")]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    watch_pid: Option<i32>,
    /// Stable tmux pane id from $TMUX_PANE (e.g. "%5"). Present only when the
    /// hook fires inside a tmux session.
    #[serde(default)]
    tmux_pane: Option<String>,
    /// Value of $TMUX (socket path, session id, pane id). Used in meta JSON.
    #[serde(default)]
    tmux_socket: Option<String>,
    /// Harness-native resume token, supplied explicitly by programmatic hosts
    /// (opencode forwards its `ses_*` id here). For claude-code/codex this is
    /// absent — their adopted `session_id` IS the resume token (see below).
    #[serde(default)]
    resume_id: Option<String>,
    /// Which harness produced this hook (`claude-code`|`codex`|`opencode`). When
    /// absent, it is inferred from the id/resume shape for alias namespacing.
    #[serde(default)]
    harness: Option<String>,
}

async fn rpc_session_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
    progress: Option<InitProgress>,
) -> Result<serde_json::Value> {
    if let Some(prog) = &progress {
        prog.emit("session_start", "parsing hook payload");
    }
    let p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    if let Some(prog) = &progress {
        prog.emit(
            "identity",
            format!("loading local key for agent {}", p.agent),
        );
    }
    let id = identity::load_or_create(&edge, &p.agent, now_secs())?;
    let cwd = p
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = crate::project::resolve(&cwd);
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let now = now_secs();
    if let Some(prog) = &progress {
        prog.emit(
            "project",
            format!("resolved project {project} from {}", cwd.display()),
        );
    }

    // Normalize the hook's identity inputs. claude-code/codex adopt their native
    // `session_id` (it doubles as the resume token); opencode supplies no
    // `session_id` and forwards its `ses_*` resume token instead. The harness
    // label is explicit when sent, else inferred from that shape (alias namespace
    // only — identity is the daemon-minted canonical id, never the harness id).
    let harness_session_id = p.session_id.clone().filter(|s| !s.is_empty());
    let resume_id = p.resume_id.clone().filter(|s| !s.is_empty());
    let harness = p
        .harness
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(Harness::from_str)
        .unwrap_or_else(|| {
            if resume_id.is_some() {
                Harness::Opencode
            } else if harness_session_id.is_some() {
                Harness::ClaudeCode
            } else {
                Harness::Unknown
            }
        });
    let tmux_pane = p.tmux_pane.clone().filter(|s| !s.is_empty());

    let obs = SessionObservation {
        agent_slug: p.agent.clone(),
        agent_pubkey: id.pubkey_hex(),
        project: project.clone(),
        host: state.host.clone(),
        rel_cwd: rel_cwd.clone(),
        harness,
        harness_session_id: harness_session_id.clone(),
        resume_id: resume_id.clone(),
        tmux_pane: tmux_pane.clone(),
        watch_pid: p.watch_pid,
        observed_at: now,
    };
    if let Some(prog) = &progress {
        prog.emit("session_registry", "registering or reasserting session");
    }
    // Canonical identity: the daemon MINTS a stable session id; the harness id /
    // resume token / pane / pid become rows in `session_aliases`. A reused
    // pane/pid slot occupied by a *different* logical session supersedes the old
    // one inside the registry (session_state lifecycle). NEVER adopt the raw
    // harness id as the identity.
    let snapshot = state.with_store(|s| s.register_or_reassert_session(&obs))?;
    let session_id = snapshot.session_id.as_str().to_owned();
    if let Some(prog) = &progress {
        prog.emit(
            "session_registry",
            format!(
                "session {} registered",
                crate::util::session_codename(&session_id)
            ),
        );
    }
    // The registration enqueued a kind:30315 publish — nudge the drainer.
    state.status_outbox_notify.notify_waiters();

    // The resume token survives the session going dead so a later `tmux resume`
    // can reconstitute the harness: opencode's `ses_*`, else claude/codex native id.
    let resume_token: Option<String> = resume_id.clone().or_else(|| harness_session_id.clone());

    // A new logical session arriving on the SAME watched pid OR tmux pane (same
    // agent/project/host) means the harness restarted without a session-end. The
    // registry already superseded the stale `session_state` row; here we cancel
    // its engine task and mark its kept `sessions` runtime row dead so `who`
    // doesn't show ghosts.
    {
        let alive = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
        let mut stale_ids: Vec<String> = Vec::new();
        for rec in &alive {
            if rec.session_id == session_id
                || rec.agent_slug != p.agent
                || rec.project != project
                || rec.host != state.host
            {
                continue;
            }
            let same_pid = p.watch_pid.is_some() && rec.watch_pid == p.watch_pid;
            let same_pane = tmux_pane.as_deref().is_some_and(|pane| {
                state
                    .with_store(|s| s.get_session_endpoint(&rec.session_id, "tmux"))
                    .ok()
                    .flatten()
                    .map(|e| e.target)
                    .as_deref()
                    == Some(pane)
            });
            if same_pid || same_pane {
                stale_ids.push(rec.session_id.clone());
            }
        }
        for old_id in stale_ids {
            cancel_session(state, &old_id);
            state.with_store(|s| {
                s.end_session(&old_id, now).ok();
                s.mark_session_dead(&old_id).ok();
            });
        }
    }

    // Atomic spawn reservation in the kept `sessions` runtime table, keyed by the
    // canonical id. This row carries the runtime-only detail (watch_pid, endpoints)
    // that `session_state` does not, and gates the idempotent re-start check below.
    state.with_store(|s| {
        s.upsert_session(&crate::state::SessionRecord {
            session_id: session_id.clone(),
            agent_slug: p.agent.clone(),
            agent_pubkey: id.pubkey_hex(),
            project: project.clone(),
            host: state.host.clone(),
            child_pid: None,
            watch_pid: p.watch_pid,
            created_at: now,
            alive: true,
            rel_cwd: rel_cwd.clone(),
        })
        .ok();
        s.touch_session(&session_id, now).ok();
        // Persist the resume token (no-op when None/empty).
        if let Some(ref rt) = resume_token {
            s.set_session_resume_id(&session_id, rt).ok();
        }
        // Record the absolute path for this project so the tmux spawn command
        // can cd to it.
        s.upsert_project_path(&project, &cwd.to_string_lossy(), now)
            .ok();
        // Register the tmux endpoint if the hook env supplied TMUX_PANE.
        if let Some(ref pane) = tmux_pane {
            let meta = serde_json::json!({
                "socket": p.tmux_socket.as_deref().unwrap_or(""),
                "pane_command": p.agent,
            })
            .to_string();
            s.upsert_session_endpoint(&session_id, "tmux", pane, &meta, now)
                .ok();
        }
    });

    // A session may acquire or refresh its tmux endpoint after unread rows were
    // already stored. Ring from the daemon on endpoint registration too, not
    // only from inbox write paths, so delivery does not depend on the tmux TUI
    // running or on a later mention event.
    if tmux_pane.is_some() {
        crate::tmux::ring_doorbells(state.clone());
    }

    // Idempotent re-start (session reassert): the engine task already runs.
    if state.sessions.lock().unwrap().contains_key(&session_id) {
        if let Some(prog) = &progress {
            prog.emit("session_start", "existing engine is already running");
        }
        return Ok(serde_json::json!({
            "session_id": session_id,
            "codename": crate::util::session_codename(&session_id),
        }));
    }

    // Make sure the project's NIP-29 group exists and this agent is a member
    // BEFORE the engine starts publishing, so its presence lands in a group it
    // already belongs to. Best-effort: never block a session from starting.
    if let Some(prog) = &progress {
        prog.emit(
            "nip29",
            "checking NIP-29 group state and membership on the relay",
        );
    }
    if let Some(init_progress) = progress.clone() {
        state
            .provider
            .open_project_with_progress(&project, &id.pubkey_hex(), move |message| {
                init_progress.emit("nip29", message);
            })
            .await;
    } else {
        state
            .provider
            .open_project(&project, &id.pubkey_hex())
            .await;
    }
    // Keep the relay-authored group state (39000/39001/39002) subscribed so the
    // membership cache stays current — "check which groups we own at all times".
    if let Some(prog) = &progress {
        prog.emit(
            "subscription",
            "opening or refreshing project subscriptions",
        );
    }
    if let Err(e) = ensure_subscription(state, &project).await {
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[daemon] ensure_subscription({project}) failed: {e:#}");
        }
        if let Some(prog) = &progress {
            prog.emit(
                "subscription",
                format!("subscription setup failed but session will continue: {e:#}"),
            );
        }
    } else if let Some(prog) = &progress {
        prog.emit("subscription", "project subscription is active");
    }

    let ep = engine_params_for(
        &state.cfg,
        &id,
        &p.agent,
        &session_id,
        &project,
        &rel_cwd,
        p.watch_pid,
    );
    if let Some(prog) = &progress {
        prog.emit("engine", "starting session engine and initial publishers");
    }
    spawn_session(state, ep).await?;
    if let Some(prog) = &progress {
        prog.emit("engine", "session engine started");
    }

    state.emit_tail(TailEvent::Sess {
        ts: now_secs(),
        project: project.clone(),
        agent: p.agent.clone(),
        session: session_id.clone(),
        state: "start".into(),
        rel_cwd: rel_cwd.clone(),
    });

    // If this pane was created by spawn-on-send, it was tagged with the mention
    // that triggered it. Type that message straight into the new session as its
    // first prompt (the whole reason the session exists). Manual spawns from the
    // TUI tag nothing and so start clean — no prompt injected. Consuming here
    // ensures injection fires exactly once regardless of call count.
    let pending_spawn = p
        .tmux_pane
        .as_deref()
        .filter(|pane| !pane.is_empty())
        .and_then(crate::tmux::consume_pending_spawn);

    if let (Some(ps), Some(pane)) = (pending_spawn, p.tmux_pane.clone()) {
        let m = ps.mention;

        // Persist the mention as already-delivered: the row lets `inbox reply
        // --id` resolve the original we're about to show, but marking it
        // delivered keeps the turn-start drain from re-injecting it as
        // duplicate context (we are typing it in directly).
        state.with_store(|s| {
            s.enqueue_mention_delivered(
                &crate::state::InboxRow {
                    mention_event_id: m.event_id.clone(),
                    target_session: session_id.clone(),
                    from_pubkey: m.from_pubkey.clone(),
                    from_slug: m.from_slug.clone(),
                    project: m.project.clone(),
                    body: m.body.clone(),
                    created_at: m.created_at,
                    from_session: m.from_session.clone(),
                    subject: String::new(),
                    branch: String::new(),
                    commit: String::new(),
                    dirty: 0,
                    host: String::new(),
                },
                now_secs(),
            )
            .ok()
        });

        // Render the received message exactly as the inbox would (provenance,
        // reply ID, body) and type it into the pane as the first prompt.
        let now = now_secs();
        let prompt = crate::cli::format_envelope(&crate::cli::EnvelopeView {
            from_slug: &m.from_slug,
            project: &m.project,
            from_session: &m.from_session,
            host: "",
            self_host: "",
            subject: "",
            branch: "",
            commit: "",
            dirty: 0,
            id: &crate::cli::mention_short_id(&m.event_id),
            sent_at: m.created_at,
            now,
            body: &m.body,
        });

        tokio::spawn(async move {
            if let Err(e) = crate::tmux::inject_spawn_message(&pane, &prompt).await {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!("[tmux] spawn message inject failed for pane {pane}: {e:#}");
                }
            } else if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] spawn message injected into pane {pane}");
            }
        });
    }

    Ok(serde_json::json!({
        "session_id": session_id,
        "codename": crate::util::session_codename(&session_id),
    }))
}

#[derive(serde::Deserialize)]
struct SessionEndParams {
    session: String,
}

fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let rec = state.with_store(|s| s.get_session(&p.session).ok().flatten());
    let existed = rec.is_some();
    if let Some(ref rec) = rec {
        // Use the canonical id (rec.session_id), NOT the raw harness id p.session:
        // the runtime handle, the session_state row, and the registry are all keyed
        // by canonical — ending by alias would cancel/end nothing.
        cancel_session(state, &rec.session_id);
        state.with_store(|s| {
            // Finish the canonical aggregate (lifecycle=ended; title retained) so
            // the session surfaces as a 'gone' delta, AND mark the kept runtime row
            // dead. The final publish carries a fresh expiration and ages off.
            s.end_session(&rec.session_id, now_secs()).ok();
            s.mark_session_dead(&rec.session_id).ok();
        });
        state.status_outbox_notify.notify_waiters();
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            project: rec.project.clone(),
            agent: rec.agent_slug.clone(),
            session: rec.session_id.clone(),
            state: "end".into(),
            rel_cwd: rec.rel_cwd.clone(),
        });
    }
    Ok(serde_json::json!({ "ended": existed }))
}

// ── send_message ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SendMessageParams {
    recipient: String,
    /// Addressing mode: `"new_session"` (recipient is an agent slug to spawn) or
    /// `"session"` (recipient is an existing session id/codename). Defaults to
    /// `"session"` for back-compat with any caller that omits it.
    #[serde(default = "default_send_mode")]
    mode: String,
    /// Project for `new_session` mode; defaults to the sender's project.
    #[serde(default)]
    project: Option<String>,
    message: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    /// Optional canonical thread id to reply into.  When Some, the provider
    /// encodes a NIP-10 root `e` tag pointing at the thread's relay-native key
    /// so the recipient materializer groups the reply into the same thread.
    /// Default: None → new thread root (Phase 6 behavior preserved).
    #[serde(default)]
    thread_id: Option<String>,
}

fn default_send_mode() -> String {
    "session".to_string()
}

async fn rpc_send_message(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::provider::SendIntent;

    let p: SendMessageParams =
        serde_json::from_value(params.clone()).context("parsing send_message params")?;
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;

    // Resolve the recipient and decide whether this send spawns a fresh session.
    // Two explicit modes (from the `--to-new-session` / `--to-session` flags):
    //   • new_session: `recipient` is an agent slug. Mint that agent's stable
    //     identity (works even if it has never run here), target NO existing
    //     session, and spawn a new harness window — the message is pre-loaded
    //     into the new session's inbox via the pending-spawn mention.
    //   • session: `recipient` is an existing session id/codename. Require that
    //     it resolves to a live session; never spawn.
    let (recipient, spawn_new): (ResolvedRecipient, Option<(String, String)>) =
        if p.mode == "new_session" {
            let agent_slug = p.recipient.clone();
            let project = p.project.clone().unwrap_or_else(|| rec.project.clone());
            let target_id =
                identity::load_or_create(&config::edge_home(), &agent_slug, now_secs())
                    .with_context(|| format!("no such agent {agent_slug:?}"))?;
            let resolved = ResolvedRecipient {
                pubkey: target_id.pubkey_hex(),
                target_session: None,
                project: project.clone(),
            };
            (resolved, Some((agent_slug, project)))
        } else {
            // `session` mode (the default): resolve the recipient as-is. The CLI
            // only ever passes a session id/codename here (`--to-session`), but the
            // RPC stays flexible — a raw pubkey/`slug@project` still resolves
            // untargeted, which is the path remote inbound mentions exercise. Never
            // spawns.
            let resolved = state.with_store(|s| resolve_recipient(s, &rec.project, &p.recipient))?;
            (resolved, None)
        };

    // Keep the message body accessible after the intent is built.
    let body = p.message.clone();

    // Build the intent. project comes from the resolved recipient (matching the
    // Mention field today), not from rec.project.
    let meta = workspace_meta(state, p.cwd.as_deref(), p.subject.unwrap_or_default(), None);
    let intent = SendIntent {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: recipient.pubkey.clone(),
        project: recipient.project.clone(),
        body: p.message,
        target_session: recipient.target_session.clone(),
        from_session: Some(rec.session_id.clone()),
        thread_id: p.thread_id.clone(),
        meta: meta.clone(),
    };

    // Publish + canonical dual-write. On error the error propagates unchanged.
    let receipt = state.provider.send(intent, &id.keys).await?;

    // LOCAL DELIVERY (the same-machine fix). When the recipient is an agent this
    // daemon hosts (e.g. a SIBLING claude session sharing the sender's pubkey),
    // delivery must NOT depend on the relay echoing our own published event back
    // to us — relays generally do not re-deliver an event to the connection that
    // published it. Route the mention into the recipient's session inbox(es) here,
    // keyed by the SAME EventId we just published. `route_mention_into` →
    // `enqueue_mention` is idempotent on `(mention_event_id, target_session)`, so
    // if the relay does echo it later, no duplicate is created. `compute_targets`
    // delivers only to the TARGET session (or all of the recipient agent's
    // sessions when untargeted) — never back to the authoring session.
    // Emit Msg event for the outbound send.
    let thread_short = pubkey_short(&receipt.thread_id);
    let to_slug = state
        .with_store(|s| s.resolve_slug_for_pubkey(&recipient.pubkey))
        .ok()
        .flatten()
        .unwrap_or_else(|| pubkey_short(&recipient.pubkey));
    state.emit_tail(TailEvent::Msg {
        ts: now_secs(),
        project: recipient.project.clone(),
        from: rec.agent_slug.clone(),
        from_session: Some(rec.session_id.clone()),
        to: to_slug,
        to_session: recipient.target_session.clone(),
        thread: Some(thread_short.clone()),
        body: body.chars().take(200).collect(),
    });

    // Emit Sync: local delivery = delivered; remote = accepted.
    let is_local = state
        .hosted_pubkeys()
        .iter()
        .any(|h| h == &recipient.pubkey);
    let sync_state = if is_local { "delivered" } else { "accepted" };
    state.emit_tail(TailEvent::Sync {
        ts: now_secs(),
        project: recipient.project.clone(),
        from: rec.agent_slug.clone(),
        to: pubkey_short(&recipient.pubkey),
        thread: Some(thread_short),
        state: sync_state.into(),
        detail: None,
    });

    // Skip same-machine fan-out for new-session sends: the message belongs to the
    // session we are about to spawn (delivered via the pending-spawn mention),
    // not to the agent's other live sessions.
    if is_local && spawn_new.is_none() {
        // Reconstruct the Mention for the legacy local-delivery path. Fields
        // must be byte-identical to what provider.send encoded and published.
        let mention = Mention {
            from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
            to_pubkey: recipient.pubkey.clone(),
            project: recipient.project.clone(),
            body: body.clone(),
            target_session: recipient.target_session.clone().map(SessionId::from),
            from_session: Some(SessionId::from(rec.session_id.clone())),
            meta,
        };
        let routed = state.with_store(|s| {
            route_mention_into_with_id(
                s,
                &recipient.pubkey,
                &mention,
                &receipt.native_event_id,
                now_secs(),
            )
        });
        if routed {
            crate::tmux::ring_doorbells(state.clone());
        }
    }

    // NEW-SESSION SPAWN: `--to-new-session <agent>` spawns a fresh harness window
    // and pre-loads the triggering mention into the new session's inbox (consumed
    // when the harness fires `session-start`). Awaited (not detached) so a spawn
    // failure — e.g. an unknown/unspawnable agent — surfaces to the caller.
    if let Some((slug, project2)) = spawn_new {
        let pending_mention = crate::tmux::PendingMention {
            event_id: receipt.native_event_id.clone(),
            from_pubkey: id.pubkey_hex(),
            from_slug: rec.agent_slug.clone(),
            from_session: rec.session_id.clone(),
            project: recipient.project.clone(),
            body: body.clone(),
            created_at: now_secs(),
        };
        let pane_id = crate::tmux::spawn_agent(state, &slug, &project2, Vec::new())
            .await
            .with_context(|| format!("spawning new session for {slug}@{project2}"))?;
        crate::tmux::register_pending_spawn_with_mention(&pane_id, pending_mention);
    }

    Ok(
        serde_json::json!({ "to_pubkey": recipient.pubkey, "target_session": recipient.target_session }),
    )
}

// ── chat_write ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ChatWriteParams {
    message: String,
    #[serde(default)]
    mention: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

async fn rpc_chat_write(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: ChatWriteParams =
        serde_json::from_value(params.clone()).context("parsing chat_write params")?;
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let from_pubkey = id.pubkey_hex();

    let mention = if let Some(raw) = p.mention.as_deref().filter(|m| !m.is_empty()) {
        let target = state.with_store(|s| resolve_recipient(s, &rec.project, raw))?;
        let Some(session_id) = target.target_session else {
            anyhow::bail!("--mention must name a concrete session id/code from `tenex-edge who`");
        };
        if target.project != rec.project {
            anyhow::bail!(
                "--mention target is in project {:?}, but this chat is for project {:?}",
                target.project,
                rec.project
            );
        }
        Some((target.pubkey, session_id))
    } else {
        None
    };
    let mentioned_pubkey = mention.as_ref().map(|(pk, _)| pk.clone());
    let mentioned_session = mention.as_ref().map(|(_, sid)| sid.clone());

    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(from_pubkey.clone(), rec.agent_slug.clone()),
        project: rec.project.clone(),
        body: p.message.clone(),
        from_session: Some(SessionId::from(rec.session_id.clone())),
        mentioned_session: mentioned_session.clone().map(SessionId::from),
        mentioned_pubkey: mentioned_pubkey.clone(),
    };
    let event_id = state
        .provider
        .publish_checked(&DomainEvent::ChatMessage(chat), &id.keys)
        .await?;
    let event_id = event_id.to_hex();
    let created_at = now_secs();

    // Local live delivery: relays often don't echo an event back to the same
    // connection that published it, and chat intentionally does not catch up old
    // history. Route now to sessions already alive in the same project.
    let routed = state.with_store(|s| {
        let _ = s.record_chat(&ChatLogRow {
            chat_event_id: event_id.clone(),
            from_pubkey: from_pubkey.clone(),
            from_slug: rec.agent_slug.clone(),
            host: state.host.clone(),
            project: rec.project.clone(),
            body: p.message.clone(),
            created_at,
            from_session: rec.session_id.clone(),
            mentioned_session: mentioned_session.clone().unwrap_or_default(),
        });
        let mut routed = false;
        for target in s.list_alive_sessions().unwrap_or_default() {
            if target.project != rec.project {
                continue;
            }
            if target.created_at > created_at {
                continue;
            }
            if target.session_id == rec.session_id {
                continue;
            }
            let row = ChatInboxRow {
                chat_event_id: event_id.clone(),
                target_session: target.session_id,
                from_pubkey: from_pubkey.clone(),
                from_slug: rec.agent_slug.clone(),
                project: rec.project.clone(),
                body: p.message.clone(),
                created_at,
                from_session: rec.session_id.clone(),
                mentioned_session: mentioned_session.clone().unwrap_or_default(),
            };
            if s.enqueue_chat(&row).unwrap_or(false) {
                routed = true;
            }
        }
        routed
    });
    if routed {
        crate::tmux::ring_doorbells(state.clone());
    }

    state.emit_tail(TailEvent::Msg {
        ts: created_at,
        project: rec.project.clone(),
        from: rec.agent_slug,
        from_session: Some(rec.session_id),
        to: mentioned_pubkey
            .as_deref()
            .map(pubkey_short)
            .unwrap_or_else(|| "project-chat".to_string()),
        to_session: mentioned_session.clone(),
        thread: None,
        body: p.message.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": event_id,
        "project": rec.project,
        "mentioned_pubkey": mentioned_pubkey,
        "mentioned_session": mentioned_session,
    }))
}

// ── propose ───────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ProposeParams {
    title: String,
    body: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
    /// Stable `d` identifier. When Some, the kind:30023 supersedes any prior
    /// proposal with the same (author, d) — a revision. When None, mint one.
    #[serde(default)]
    d: Option<String>,
}

/// Publish a kind:30023 (NIP-23 long-form) proposal signed by the agent's identity.
///
/// Tags:
///   ["d", <short-id>]           — addressable identifier (NIP-33)
///   ["title", <title>]          — human-readable title
///   ["h", <project>]            — NIP-29 group
///   ["p", <owner>]              — per owner in cfg.owners, surfaces to the human
///   ["e", <root>, "", "root"]   — only when --thread given; links to work-thread
///   ["session-id", <session>]   — authoring session, lets a note route back
///   (no agent tag — author identity is the event signer pubkey; kind:0 carries slug)
///
/// Dual-writes a canonical row: project_origin → thread_origin (thread_id or
/// the proposal's own event id as a new root) → message (direction=outbound,
/// sync_state=published, body=title).
async fn rpc_propose(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::provider::FABRIC;

    let p: ProposeParams =
        serde_json::from_value(params.clone()).context("parsing propose params")?;
    if p.title.is_empty() {
        anyhow::bail!("title must not be empty");
    }

    // Resolve session if one is live; fall back to cwd-based project + env agent.
    // propose doesn't require a live session — it just needs a project and a key.
    let session_rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )
    .ok();
    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = session_rec
        .as_ref()
        .map(|r| r.project.clone())
        .unwrap_or_else(|| crate::project::resolve(&cwd));
    let agent_slug = session_rec
        .as_ref()
        .map(|r| r.agent_slug.clone())
        .or_else(|| p.agent.clone().filter(|a| !a.is_empty()))
        .unwrap_or_else(|| "agent".to_string());
    let id = identity::load_or_create(&config::edge_home(), &agent_slug, now_secs())?;

    // Addressable `d` identifier. A caller-supplied `d` makes this a REVISION
    // that supersedes the prior (author, d) at the same naddr; otherwise mint one.
    let d_tag = p.d.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| {
        format!(
            "prop-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        )
    });

    // Resolve the thread root native key if --thread given.
    let root_native_key: Option<String> = p.thread_id.as_deref().and_then(|tid| {
        state.with_store(|s| {
            s.thread_root_native_key(tid, FABRIC, &state.provider.provider_instance)
        })
    });

    // Build the Proposal domain event; the wire shape lives in the codec.
    let ev = DomainEvent::Proposal(crate::domain::Proposal {
        agent: crate::domain::AgentRef::new(id.pubkey_hex(), agent_slug.clone()),
        project: project.clone(),
        title: p.title.clone(),
        body: p.body.clone(),
        d: d_tag.clone(),
        // Authoring session — only when a live session exists.
        session_id: session_rec
            .as_ref()
            .map(|rec| crate::util::SessionId::from(rec.session_id.clone())),
        // Surface to each owner.
        audience: state.owners.clone(),
        thread_root_key: root_native_key,
    });
    // Checked publish: a NIP-29 relay rejecting the kind:30023 (e.g. the author
    // isn't a member of the project group) used to resolve Ok and report a false
    // "published" — silent data loss. `publish_checked` fails on relay rejection
    // so the CLI exits nonzero with the relay's stated reason.
    let event_id = state
        .provider
        .publish_checked(&ev, &id.keys)
        .await
        .context("publishing proposal")?;
    let eid_hex = event_id.to_hex();

    // Internal read-back: confirm the event is actually retrievable from the
    // relay, not merely accepted. Surfaces a relay that ACKs writes but silently
    // drops them. Best-effort and non-fatal — reported to the caller so it can
    // warn loudly without failing a publish the relay genuinely accepted.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let retrievable = state
        .provider
        .is_retrievable(event_id, Duration::from_secs(5))
        .await;

    // Dual-write canonical read-model rows.
    let now = now_secs();
    let pi = state.provider.provider_instance.clone();
    let thread_id = state.with_store(|s| -> Result<String> {
        let project_id = s.ensure_project_origin(FABRIC, &pi, &project, &project, now)?;
        let thread_id = if let Some(tid) = p.thread_id.as_deref() {
            // Attach to an existing thread.
            // ensure_thread_origin is idempotent; use the proposal's event id as
            // native key for this message within the thread.
            s.ensure_thread_origin(&project_id, FABRIC, &pi, tid, now)?;
            tid.to_string()
        } else {
            // New standalone thread rooted at the proposal's event id.
            s.ensure_thread_origin(&project_id, FABRIC, &pi, &eid_hex, now)?
        };
        // Record the proposal as an outbound message; body = title (full body is on relay).
        let msg_id = s.record_message(
            &thread_id,
            &id.pubkey_hex(),
            &p.title,
            now,
            "outbound",
            "published",
            Some(&eid_hex),
        )?;
        // Owner as recipient (so they see it in threads).
        for owner in &state.owners {
            s.add_message_recipient(&msg_id, owner, None)?;
        }
        Ok(thread_id)
    })?;

    Ok(serde_json::json!({
        "event_id": eid_hex,
        "d_tag": d_tag,
        "thread_id": thread_id,
        "title": p.title,
        "retrievable": retrievable,
    }))
}

struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

fn resolve_recipient(store: &Store, my_project: &str, target: &str) -> Result<ResolvedRecipient> {
    if let Some((slug, proj)) = target.split_once('@') {
        let pk = store
            .resolve_agent_pubkey(slug, Some(proj))?
            .with_context(|| {
                format!("can't resolve {slug}@{proj} (no presence/profile seen yet)")
            })?;
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: proj.to_string(),
        });
    }
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(ResolvedRecipient {
            pubkey: target.to_string(),
            target_session: None,
            project: my_project.to_string(),
        });
    }
    // Exact session id, or a harness external id (claude/codex native id,
    // opencode resume token, tmux pane, watch pid) resolvable to canonical via
    // `session_aliases`. Checked before prefix matching so a full id always wins.
    if let Some(s) = store.get_session(target)? {
        return Ok(ResolvedRecipient {
            pubkey: s.agent_pubkey,
            target_session: Some(s.session_id),
            project: s.project,
        });
    }
    if target.len() >= 6 {
        if let Some(ps) = store.find_peer_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: ps.pubkey,
                target_session: Some(ps.session_id),
                project: ps.project,
            });
        }
        if let Some(s) = store.find_session_by_prefix(target)? {
            return Ok(ResolvedRecipient {
                pubkey: s.agent_pubkey,
                target_session: Some(s.session_id),
                project: s.project,
            });
        }
        // Try matching against session codenames (from `who` display). This is a
        // fallback for when users copy a session codename from `who` output.
        if let Some(found) = find_session_by_codename(store, target)? {
            return Ok(ResolvedRecipient {
                pubkey: found.pubkey,
                target_session: Some(found.session_id),
                project: found.project,
            });
        }
    }
    if let Some(pk) = store.resolve_agent_pubkey(target, Some(my_project))? {
        return Ok(ResolvedRecipient {
            pubkey: pk,
            target_session: None,
            project: my_project.to_string(),
        });
    }
    anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
}

struct SessionMatch {
    pubkey: String,
    session_id: String,
    project: String,
}

/// Try to find a session (peer or own) matching the given codename.
/// Codenames are what `who` displays for sessions (e.g. `bravo4217`).
fn find_session_by_codename(store: &Store, codename: &str) -> Result<Option<SessionMatch>> {
    let target_code = codename.to_lowercase();

    // Search peer sessions
    if let Ok(peers) = store.list_peer_sessions(None, 0) {
        for peer in peers {
            if session_codename(&peer.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: peer.pubkey,
                    session_id: peer.session_id,
                    project: peer.project,
                }));
            }
        }
    }

    // Search own sessions
    if let Ok(sessions) = store.list_my_live_sessions(0) {
        for session in sessions {
            if session_codename(&session.session_id).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: session.agent_pubkey,
                    session_id: session.session_id,
                    project: session.project,
                }));
            }
        }
    }

    Ok(None)
}

// ── inbox / turn_start / turn_check / turn_end ───────────────────────────────

#[derive(serde::Deserialize, Default)]
struct InboxParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

async fn rpc_inbox(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: InboxParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let _ = fetch_mentions_into_inbox(state, &rec).await;

    let rows = state.with_store(|s| {
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        for r in &rows {
            s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                .ok();
        }
        rows
    });
    let rows_json = rows_to_json(&rows, &state.host);

    Ok(serde_json::json!({
        "rows": rows_json,
    }))
}

#[derive(serde::Deserialize, Default)]
struct TurnStartParams {
    session: String,
    #[serde(default)]
    transcript: Option<String>,
}

async fn rpc_turn_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnStartParams =
        serde_json::from_value(params.clone()).context("parsing turn_start params")?;
    if p.session.is_empty() {
        return Ok(serde_json::json!({ "context": serde_json::Value::Null }));
    }
    // Hooks speak the harness id; resolve to the canonical session_state id or the
    // transition below updates ZERO rows (harness id is only an alias). This is the
    // single owner of the turn-start transition — the runtime engine only OBSERVES
    // turn_state and never opens/closes the turn itself.
    let session = state.with_store(|s| s.canonical_session_id(&p.session));

    let prev_started = state.with_store(|s| {
        let (_, prev) = s.get_turn_state(&session).unwrap_or((false, 0));
        // Canonical transition: busy=1, turn_id+1, activity cleared, version bump +
        // status_outbox enqueue. Also writes turn_state so turn_check_due() works.
        s.start_turn(&session, now_secs()).ok();
        if let Some(path) = p.transcript.as_deref().filter(|x| !x.is_empty()) {
            s.set_session_transcript(&session, path).ok();
            // Snapshot the last assistant text so rpc_turn_end can poll until a
            // *new* (different) response appears — Claude Code writes the
            // transcript after the stop hook fires, so reading at stop time often
            // returns the previous turn's content.
            let baseline = crate::transcript::read_last_assistant_text(std::path::Path::new(path))
                .unwrap_or_default();
            s.set_last_assistant_text_at_turn_start(&session, &baseline)
                .ok();
        }
        prev
    });
    state.status_outbox_notify.notify_waiters();

    let rec = match state.with_store(|s| s.get_session(&session).ok().flatten()) {
        Some(r) => r,
        None => return Ok(serde_json::json!({ "context": serde_json::Value::Null })),
    };

    // Emit Turn{working} for the live tail feed.
    state.emit_tail(TailEvent::Turn {
        ts: now_secs(),
        project: rec.project.clone(),
        agent: rec.agent_slug.clone(),
        session: rec.session_id.clone(),
        state: "working".into(),
        elapsed_s: None,
    });

    // Self-fetch stored mentions (relay), then assemble via the SHARED cli.rs
    // function so the injected text is byte-identical to the pre-daemon CLI and
    // cannot drift.
    let _ = fetch_mentions_into_inbox(state, &rec).await;
    let context = crate::cli::assemble_turn_start_context(&state.store, &rec, prev_started)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize, Default)]
struct TurnCheckParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

fn rpc_turn_check(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnCheckParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let now = now_secs();
    // Rate-limit the sibling-session delta to at most once per 60s per session
    // (the cursor write is safe: the daemon is the single store writer). `None`
    // → the floor hasn't passed (or not mid-turn), so only the inbox peek runs.
    let delta_since =
        state.with_store(|s| s.turn_check_due(&rec.session_id, now, 60).unwrap_or(None));
    let context =
        crate::cli::assemble_turn_check_context(&state.store, &rec, &state.host, delta_since, now)
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize)]
struct TurnEndParams {
    session: String,
}

async fn rpc_turn_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: TurnEndParams =
        serde_json::from_value(params.clone()).context("parsing turn_end params")?;
    if !p.session.is_empty() {
        // Hooks speak the harness id; resolve to canonical or end_turn no-ops.
        // Single owner of the turn-end transition (runtime only observes).
        let session = state.with_store(|s| s.canonical_session_id(&p.session));
        // Read turn_started_at BEFORE marking end, so we can compute elapsed.
        // Thread IDs are captured NOW so a concurrent user_prompt for the next
        // turn cannot overwrite last_prompt_event_id before we publish.
        let (was_working, turn_started_at) =
            state.with_store(|s| s.get_turn_state(&session).unwrap_or((false, 0)));
        let (root_event_id, last_prompt_event_id, transcript_path, baseline_text) = state
            .with_store(|s| {
                // Canonical transition: busy=0, activity cleared, TITLE retained,
                // version bump + status_outbox enqueue. Also clears turn_state.
                s.end_turn(&session, now_secs()).ok();
                let (root, prompt) = s.get_thread_event_ids(&session);
                let transcript = s.get_session_transcript(&session).ok().flatten();
                let baseline = s.get_last_assistant_text_at_turn_start(&session);
                (root, prompt, transcript, baseline)
            });
        state.status_outbox_notify.notify_waiters();

        // Publish the NIP-10 TurnReply when we have full threading context.
        if !root_event_id.is_empty() && !last_prompt_event_id.is_empty() {
            if let Some(rec) = state.with_store(|s| s.get_session(&session).ok().flatten()) {
                // Claude Code writes the transcript *after* the stop hook fires, so
                // the response may not be on disk yet. Poll (up to ~2 s) until the
                // last assistant text differs from what we snapshotted at turn_start.
                let body = if let Some(path) = transcript_path.as_deref() {
                    let mut result = String::new();
                    for _ in 0..20u8 {
                        if let Some(text) =
                            crate::transcript::read_last_assistant_text(std::path::Path::new(path))
                        {
                            if !text.is_empty() && text != baseline_text {
                                result = text;
                                break;
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                    result
                } else {
                    String::new()
                };

                if !body.is_empty() {
                    let ev = DomainEvent::TurnReply(crate::domain::TurnReply {
                        agent: crate::domain::AgentRef::new(
                            rec.agent_pubkey.clone(),
                            rec.agent_slug.clone(),
                        ),
                        project: rec.project.clone(),
                        body,
                        root_event_id,
                        reply_event_id: last_prompt_event_id,
                    });
                    let edge = crate::config::edge_home();
                    if let Ok(id) =
                        crate::identity::load_or_create(&edge, &rec.agent_slug, now_secs())
                    {
                        state.provider.publish(&ev, &id.keys).await.ok();
                    }
                }
            }
        }

        if was_working {
            let now = now_secs();
            let elapsed_s = if turn_started_at > 0 {
                Some(now.saturating_sub(turn_started_at))
            } else {
                None
            };
            if let Some(rec) = state.with_store(|s| s.get_session(&session).ok().flatten()) {
                state.emit_tail(TailEvent::Turn {
                    ts: now,
                    project: rec.project.clone(),
                    agent: rec.agent_slug.clone(),
                    session: rec.session_id.clone(),
                    state: "idle".into(),
                    elapsed_s,
                });
            }
        }
    }
    Ok(serde_json::json!({ "ok": true }))
}

// ── doctor ───────────────────────────────────────────────────────────────────

async fn rpc_doctor(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let relays = state.cfg.relays.clone();
    let probe = state
        .keys_for(&state.hosted_pubkeys().first().cloned().unwrap_or_default())
        .map(|k| k.public_key().to_hex());
    // The probe's wire shape lives in the provider; readers only see strings.
    let (publish, readback) = state.provider.doctor_probe().await;
    Ok(serde_json::json!({
        "relays": relays,
        "probe_pubkey": probe,
        "publish": publish,
        "readback": readback,
    }))
}

// ── user_prompt ──────────────────────────────────────────────────────────────

/// Publish a kind:1 OP signed by the human user's nsec. The event records the
/// user's prompt on the Nostr fabric as a root note (no `e` tag) in the NIP-29
/// group, p-tagging the agent that will process it.
async fn rpc_user_prompt(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        env_session: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        prompt: Option<String>,
        #[serde(default)]
        agent: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).unwrap_or_default();

    let nsec = match &state.cfg.user_nsec {
        Some(n) => n.clone(),
        None => anyhow::bail!("userNsec not set in ~/.tenex/config.json"),
    };
    let user_keys = Keys::parse(&nsec).context("parsing userNsec")?;

    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let body = p.prompt.unwrap_or_default();

    // The user's prompt is a Mention from the owner to the agent — same domain
    // event, same codec; only the signing key differs.
    let ev = DomainEvent::Mention(Mention {
        from: crate::domain::AgentRef::new(user_keys.public_key().to_hex(), String::new()),
        to_pubkey: rec.agent_pubkey.clone(),
        project: rec.project.clone(),
        body,
        target_session: None,
        from_session: None,
        meta: crate::domain::MentionMeta::default(),
    });
    // Suppress the relay echo of our own prompt: this RPC is only ever invoked
    // by the LOCAL harness's user-prompt-submit hook, so the agent already has
    // the prompt in front of it. Routing the echoed kind:1 back into this same
    // agent's inbox would create a phantom unread mention — and because the tmux
    // doorbell auto-submits its nudge text as a prompt, that echo perpetually
    // re-arms the doorbell (an infinite "you have new mentions" loop). Publishing
    // via `publish_seen_by` records the event as seen BEFORE the wire send, so
    // `route_mention_into` drops the untargeted echo even though it arrives on a
    // separate task. Remote prompts never pass through this RPC, so they're safe.
    let event_id = state
        .provider
        .publish_seen_by(&ev, &user_keys, &rec.agent_pubkey)
        .await?;

    // NIP-10 thread tracking: first prompt becomes the root; every prompt is
    // the "last trigger" the next TurnReply will reply to.
    let eid = event_id.to_hex();
    let sid = rec.session_id.clone();
    state.with_store(|s| {
        let (root, _) = s.get_thread_event_ids(&sid);
        let new_root = if root.is_empty() { eid.clone() } else { root };
        s.set_thread_event_ids(&sid, &new_root, &eid).ok();
    });

    Ok(serde_json::json!({ "event_id": eid }))
}

// ── project_list ─────────────────────────────────────────────────────────────

/// List NIP-29 groups: refresh the local cache via the provider (which fetches
/// kind:39000 from the relay), then return the read-model list.
async fn rpc_project_list(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    // Provider fetches kind:39000 from the relay and upserts project_meta.
    // Best-effort: a relay timeout must not prevent returning cached results.
    state.provider.refresh_project_list().await.ok();

    // Read the current read-model (backed by project_meta — retained storage).
    let local = state
        .with_store(|s| s.list_projects_read_model())
        .unwrap_or_default();

    let mut projects: Vec<serde_json::Value> = local
        .into_iter()
        .map(|(slug, about)| serde_json::json!({ "slug": slug, "about": about }))
        .collect();
    projects.sort_by(|a, b| {
        a["slug"]
            .as_str()
            .unwrap_or("")
            .cmp(b["slug"].as_str().unwrap_or(""))
    });

    Ok(serde_json::json!({ "projects": projects }))
}

// ── project_edit ─────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9002 (edit-metadata) event signed by the human user's
/// nsec. The relay validates admin rights and updates its kind:39000 accordingly.
async fn rpc_project_edit(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        description: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_edit params")?;

    let nsec = state
        .cfg
        .user_nsec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("userNsec not set in ~/.tenex/config.json"))?;
    let user_keys = Keys::parse(nsec).context("parsing userNsec")?;

    // NIP-29 edit-metadata: the wire shape lives in the nip29 lifecycle module.
    // The relay validates admin rights and re-publishes kind:39000.
    let builder = crate::fabric::nip29::lifecycle::group_edit_metadata(&p.project, &p.description)?;
    let event_id = state.transport.publish_signed(builder, &user_keys).await?;

    // Optimistically update local cache; the relay will also push kind:39000.
    let now = now_secs();
    state.with_store(|s| {
        s.upsert_project_meta(&p.project, &p.description, now).ok();
    });

    Ok(serde_json::json!({
        "event_id": event_id.to_hex(),
        "project": p.project,
    }))
}

// ── project_members ──────────────────────────────────────────────────────────

/// Return the cached NIP-29 membership roster for a project. Before reading the
/// cache, try to refresh kind:39002 from the relay so interactive project edits
/// start from the relay's current roster rather than only local optimistic state.
async fn rpc_project_members(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_members params")?;
    refresh_project_members_cache(state, &p.project).await;

    let members = state
        .with_store(|s| s.list_group_members(&p.project))
        .unwrap_or_default()
        .into_iter()
        .map(|(pubkey, role)| serde_json::json!({ "pubkey": pubkey, "role": role }))
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "project": p.project,
        "members": members,
    }))
}

async fn refresh_project_members_cache(state: &Arc<DaemonState>, project: &str) {
    use crate::codec::kind1::{kind, KIND_GROUP_MEMBERS};
    use nostr_sdk::prelude::Filter;

    let filter = Filter::new()
        .kind(kind(KIND_GROUP_MEMBERS))
        .identifier(project)
        .limit(5);
    let Ok(events) = state.transport.fetch(filter, Duration::from_secs(5)).await else {
        return;
    };
    let Some(ev) = events.iter().max_by_key(|e| e.created_at.as_secs()) else {
        return;
    };
    let members = ev
        .tags
        .iter()
        .filter_map(|t| {
            let s = t.as_slice();
            if s.first().map(String::as_str) != Some("p") {
                return None;
            }
            let pubkey = s.get(1)?.clone();
            let role = s.get(2).cloned().unwrap_or_else(|| "member".to_string());
            Some((pubkey, role))
        })
        .collect::<Vec<_>>();
    state.with_store(|s| {
        s.replace_group_members(project, &members, now_secs()).ok();
    });
}

// ── statusline ───────────────────────────────────────────────────────────────

/// How long a drained mention keeps showing on the statusline as "recently
/// consumed" before disappearing.
const STATUSLINE_RECENT_SECS: u64 = 30;
/// How long a distillation error stays visible in the statusline before expiring.
const DISTILL_ERROR_TTL_SECS: u64 = 300;

#[derive(serde::Deserialize, Default)]
struct StatuslineParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

/// `statusline`: everything the host's status bar renders, in one pure-read RPC.
/// Like `turn_check`, this is called constantly by the harness, so it must
/// NEVER write to state.db (no drains, no touches) — peeks only.
fn rpc_statusline(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: StatuslineParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let now = now_secs();
    let host = state.host.clone();
    state.with_store(|s| {
        let session_count = crate::cli::load_who_snapshot(s, Some(&rec.project), false, now, &host)
            .map(|snap| snap.session_count())
            .unwrap_or(0);
        let member_count = s.count_group_members(&rec.project).unwrap_or(0);
        let is_member = s
            .is_group_member(&rec.project, &rec.agent_pubkey)
            .unwrap_or(true);
        // Read busy + title from the canonical aggregate via the SHARED projection
        // (derive_status), so the statusline agrees with `who`/turn-deltas. Pure
        // read: no drains, no touches.
        let (working, status) = s
            .local_session_snapshot(&rec.session_id)
            .ok()
            .flatten()
            .map(|snap| {
                let d = derive_status(&snap, now);
                (d.busy, d.title)
            })
            .unwrap_or((false, String::new()));
        let pending = s.peek_inbox(&rec.session_id).unwrap_or_default();
        let pending_chat = s.peek_chat_mentions(&rec.session_id).unwrap_or_default();
        let recent_since = now.saturating_sub(STATUSLINE_RECENT_SECS);
        let recent = s
            .list_recently_delivered(&rec.session_id, recent_since)
            .unwrap_or_default();
        let recent_chat = s
            .list_recently_delivered_chat_mentions(&rec.session_id, recent_since)
            .unwrap_or_default();
        let mut pending_json = rows_to_json(&pending, &host);
        pending_json.extend(chat_rows_to_json(&pending_chat));
        sort_message_json(&mut pending_json);
        let mut recent_json = rows_to_json(&recent, &host);
        recent_json.extend(chat_rows_to_json(&recent_chat));
        sort_message_json(&mut recent_json);
        let distill_error = s
            .get_recent_session_error(&rec.session_id, now.saturating_sub(DISTILL_ERROR_TTL_SECS))
            .ok()
            .flatten();
        Ok(serde_json::json!({
            "agent": rec.agent_slug,
            "host": host,
            "session_id": rec.session_id,
            "project": rec.project,
            "member_count": member_count,
            "session_count": session_count,
            "is_member": is_member,
            "working": working,
            "status": status,
            "pending": pending_json,
            "recent": recent_json,
            "distill_error": distill_error,
        }))
    })
}

// ── whoami (this session's own identity) ──────────────────────────────────────

/// `whoami`: the calling session's own identity card. Resolves the current
/// session the same way `statusline`/`inbox` do (explicit → env → cwd/agent),
/// then returns who it is on the fabric: agent slug, the targetable session
/// codename, the canonical session id, project, host, pubkey (hex + npub), and
/// its current working/title status. Pure read — no writes, like `statusline`.
fn rpc_whoami(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: StatuslineParams = serde_json::from_value(params.clone()).unwrap_or_default();
    // Strict: no bare-project fallback. `whoami` answers "which agent am I", so
    // when run outside an agent (no session/agent signal) it must error, not
    // silently report some unrelated sibling session in the cwd's project.
    let rec = resolve_session_inner(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        false,
    )?;
    let now = now_secs();
    let host = state.host.clone();
    let npub = {
        use nostr_sdk::prelude::ToBech32;
        nostr_sdk::PublicKey::from_hex(&rec.agent_pubkey)
            .ok()
            .and_then(|pk| pk.to_bech32().ok())
    };
    state.with_store(|s| {
        let is_member = s
            .is_group_member(&rec.project, &rec.agent_pubkey)
            .unwrap_or(true);
        let (working, status) = s
            .local_session_snapshot(&rec.session_id)
            .ok()
            .flatten()
            .map(|snap| {
                let d = derive_status(&snap, now);
                (d.busy, d.title)
            })
            .unwrap_or((false, String::new()));
        let pending = s.peek_inbox(&rec.session_id).unwrap_or_default().len()
            + s.peek_chat_mentions(&rec.session_id)
                .unwrap_or_default()
                .len();
        Ok(serde_json::json!({
            "agent": rec.agent_slug,
            "session_id": rec.session_id,
            "codename": crate::util::session_codename(&rec.session_id),
            "project": rec.project,
            "host": host,
            "rel_cwd": rec.rel_cwd,
            "pubkey": rec.agent_pubkey,
            "npub": npub,
            "is_member": is_member,
            "working": working,
            "status": status,
            "pending": pending,
            "created_at": rec.created_at,
        }))
    })
}

// ── inbox reply (reply by mention ID) ─────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct InboxReplyParams {
    /// Short `ID` from an envelope (prefix of the original mention's event id).
    id: String,
    message: String,
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

/// Reply to a mention by its short `ID`. Looks up the original inbox row, then
/// sends through the provider a Mention that `p`-tags the original sender and
/// `e`-tags (NIP-10 reply) the original event — threading the reply back to
/// exactly the sender session that wrote it. The reply is filed into the
/// original's canonical thread, so both sides' read models agree. Subject
/// defaults to `Re: <original subject>`.
async fn rpc_inbox_reply(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::provider::SendIntent;

    let p: InboxReplyParams =
        serde_json::from_value(params.clone()).context("parsing inbox_reply params")?;
    if p.id.is_empty() {
        anyhow::bail!("missing --id (the ID shown on the message you're replying to)");
    }
    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;

    let original = state
        .with_store(|s| s.find_inbox_by_event_prefix(&p.id))?
        .with_context(|| format!("no message in this inbox with ID {:?}", p.id))?;

    // Default the subject to `Re: <original>` (don't double-prefix on a reply chain).
    let subject = match p.subject {
        Some(s) if !s.is_empty() => s,
        _ if original.subject.is_empty() => String::new(),
        _ if original.subject.to_lowercase().starts_with("re:") => original.subject.clone(),
        _ => format!("Re: {}", original.subject),
    };

    let mut meta = workspace_meta(state, p.cwd.as_deref(), subject, None);
    meta.reply_to_event_id = Some(original.mention_event_id.clone());

    // File the reply into the original's canonical thread when we know it.
    let thread_id = state.with_store(|s| s.thread_for_native_event(&original.mention_event_id));

    let intent = SendIntent {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: original.from_pubkey.clone(),
        project: original.project.clone(),
        body: p.message.clone(),
        // Route back to the precise sender session when we captured one.
        target_session: Some(original.from_session.clone()).filter(|s| !s.is_empty()),
        from_session: Some(rec.session_id.clone()),
        thread_id,
        meta: meta.clone(),
    };
    let receipt = state.provider.send(intent, &id.keys).await?;

    // Tail: outbound msg + sync, mirroring rpc_send_message.
    let thread_short = pubkey_short(&receipt.thread_id);
    let to_slug = state
        .with_store(|s| s.resolve_slug_for_pubkey(&original.from_pubkey))
        .ok()
        .flatten()
        .unwrap_or_else(|| pubkey_short(&original.from_pubkey));
    state.emit_tail(TailEvent::Msg {
        ts: now_secs(),
        project: original.project.clone(),
        from: rec.agent_slug.clone(),
        from_session: Some(rec.session_id.clone()),
        to: to_slug,
        to_session: Some(original.from_session.clone()).filter(|s| !s.is_empty()),
        thread: Some(thread_short.clone()),
        body: p.message.chars().take(200).collect(),
    });
    let is_local = state
        .hosted_pubkeys()
        .iter()
        .any(|h| h == &original.from_pubkey);
    state.emit_tail(TailEvent::Sync {
        ts: now_secs(),
        project: original.project.clone(),
        from: rec.agent_slug.clone(),
        to: pubkey_short(&original.from_pubkey),
        thread: Some(thread_short),
        state: (if is_local { "delivered" } else { "accepted" }).into(),
        detail: None,
    });

    // Local delivery to a same-machine sibling session (see rpc_send_message).
    if is_local {
        let mention = Mention {
            from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
            to_pubkey: original.from_pubkey.clone(),
            project: original.project.clone(),
            body: p.message,
            target_session: Some(original.from_session.clone())
                .filter(|s| !s.is_empty())
                .map(SessionId::from),
            from_session: Some(SessionId::from(rec.session_id.clone())),
            meta,
        };
        let routed = state.with_store(|s| {
            route_mention_into_with_id(
                s,
                &original.from_pubkey,
                &mention,
                &receipt.native_event_id,
                now_secs(),
            )
        });
        if routed {
            crate::tmux::ring_doorbells(state.clone());
        }
    }

    Ok(serde_json::json!({
        "to_pubkey": original.from_pubkey,
        "target_session": original.from_session,
        "in_reply_to": original.mention_event_id,
    }))
}

/// Capture the sender's envelope metadata: `subject` plus a snapshot of the git
/// workspace at `cwd` (branch, short commit, dirty-file count) and this daemon's
/// host. `reply_to` is left `None` here; callers set it for replies.
fn workspace_meta(
    state: &Arc<DaemonState>,
    cwd: Option<&str>,
    subject: String,
    reply_to: Option<String>,
) -> crate::domain::MentionMeta {
    let (branch, commit, dirty) = git_snapshot(cwd);
    crate::domain::MentionMeta {
        subject,
        branch,
        commit,
        dirty,
        host: state.host.clone(),
        reply_to_event_id: reply_to,
    }
}

/// `(branch, short_commit, dirty_count)` for the git repo at `cwd` (or the
/// daemon's cwd when `None`). All-empty / zero when `cwd` isn't a git repo.
/// `dirty_count` is `git status --porcelain` line count, which already excludes
/// gitignored files.
fn git_snapshot(cwd: Option<&str>) -> (String, String, u32) {
    use std::process::Command;
    let dir = cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&dir)
            .args(args)
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let branch = git(&["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = git(&["status", "--porcelain"])
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u32)
        .unwrap_or(0);
    (branch, commit, dirty)
}

// ── project_add ──────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9000 (put-user) event to add a pubkey to the group.
/// Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
async fn rpc_project_add(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_add params")?;

    let nsec = state
        .cfg
        .user_nsec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("userNsec not set in ~/.tenex/config.json"))?;
    let user_keys = Keys::parse(nsec).context("parsing userNsec")?;

    let pubkey_hex = resolve_pubkey_hex(&p.pubkey).await?;

    let builder = crate::fabric::nip29::lifecycle::group_put_user(&p.project, &pubkey_hex)?;
    state
        .transport
        .publish_signed_checked(builder, &user_keys)
        .await?;

    state.with_store(|s| {
        s.upsert_group_member(&p.project, &pubkey_hex, "member", now_secs())
            .ok();
    });

    Ok(serde_json::json!({
        "project": p.project,
        "pubkey": pubkey_hex,
    }))
}

// ── project_remove ───────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9001 (remove-user) event to remove a pubkey from the
/// group. Accepts hex, npub (bech32), or a NIP-05 address (user@domain.com).
async fn rpc_project_remove(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct P {
        project: String,
        pubkey: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("project_remove params")?;

    let nsec = state
        .cfg
        .user_nsec
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("userNsec not set in ~/.tenex/config.json"))?;
    let user_keys = Keys::parse(nsec).context("parsing userNsec")?;

    let pubkey_hex = resolve_pubkey_hex(&p.pubkey).await?;

    let builder = crate::fabric::nip29::lifecycle::group_remove_user(&p.project, &pubkey_hex)?;
    state
        .transport
        .publish_signed_checked(builder, &user_keys)
        .await?;

    state.with_store(|s| {
        let ts = now_secs();
        s.remove_group_member(&p.project, &pubkey_hex).ok();
        s.revoke_member(&p.project, &pubkey_hex, ts).ok();
    });

    Ok(serde_json::json!({
        "project": p.project,
        "pubkey": pubkey_hex,
    }))
}

// ── publish_profile ───────────────────────────────────────────────────────────

/// Publish an agent's kind:0 identity card (Profile) immediately, signed by the
/// agent's OWN keys (loaded from the local keystore by slug). Used by
/// `tenex-edge agent add` so a freshly-minted agent is discoverable on the
/// indexer relay without waiting for its first session — identical in shape to
/// the Profile the session engine publishes on session start (runtime.rs).
async fn rpc_publish_profile(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        slug: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("publish_profile params")?;

    let edge_home = crate::config::edge_home();
    let id = crate::identity::load_or_create(&edge_home, &p.slug, now_secs())
        .with_context(|| format!("loading agent {}", p.slug))?;

    let ev = DomainEvent::Profile(crate::domain::Profile {
        agent: crate::domain::AgentRef::new(id.pubkey_hex(), p.slug.clone()),
        host: state.host.clone(),
        owners: state.owners.clone(),
    });
    let event_id = state.provider.publish(&ev, &id.keys).await?;

    Ok(serde_json::json!({
        "slug": p.slug,
        "pubkey": id.pubkey_hex(),
        "event_id": event_id.to_hex(),
    }))
}

async fn resolve_pubkey_hex(input: &str) -> Result<String> {
    use nostr_sdk::prelude::PublicKey;

    // hex / npub / nostr: URI
    if let Ok(pk) = PublicKey::parse(input) {
        return Ok(pk.to_hex());
    }

    // NIP-05: name@domain
    if let Some((name, domain)) = input.split_once('@') {
        if !domain.is_empty() {
            let url = format!("https://{domain}/.well-known/nostr.json?name={name}");
            let json: serde_json::Value = reqwest::get(url)
                .await
                .with_context(|| format!("NIP-05 HTTP request to {domain} failed"))?
                .json()
                .await
                .context("NIP-05 response is not valid JSON")?;
            let hex = json["names"][name]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("NIP-05: name {name:?} not found at {domain}"))?;
            return PublicKey::from_hex(hex)
                .map(|pk| pk.to_hex())
                .context("NIP-05 returned invalid pubkey");
        }
    }

    anyhow::bail!("cannot parse {input:?} as pubkey (hex/npub) or NIP-05 (user@domain)")
}

// ── list_threads / messages / thread_meta (Phase 7 read RPCs) ────────────────

/// `list_threads`: return enriched thread list for a project.
///
/// Params: `{ "project": "<slug-or-project_id>" }` (slug resolved via
/// `project_id_for_slug` on the kind1-nip29 fabric; no-op create if unknown).
async fn rpc_list_threads(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("list_threads params")?;
    let pi = state.provider.provider_instance.clone();

    // Read-only: resolve slug → project_id without creating an origin.
    // When the project has no recorded origin yet (no message traffic; no backfill)
    // return an empty list rather than erroring — consistent with other read-model
    // methods that gracefully degrade to empty on an empty store.
    let Some(project_id) = state
        .with_store(|s| s.project_id_for_slug(crate::fabric::provider::FABRIC, &pi, &p.project))?
    else {
        return Ok(serde_json::json!([]));
    };

    let threads = state.with_store(|s| s.list_threads(&project_id))?;
    Ok(serde_json::to_value(&threads)?)
}

/// `messages`: return canonical messages for a thread.
///
/// Params: `{ "thread_id": "<thread_id>" }`
fn rpc_messages(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        thread_id: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("messages params")?;
    let msgs = state.with_store(|s| s.messages_for_thread(&p.thread_id))?;
    Ok(serde_json::to_value(&msgs)?)
}

/// `thread_meta`: return enriched metadata for a single thread.
///
/// Params: `{ "thread_id": "<thread_id>" }`
fn rpc_thread_meta(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        thread_id: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("thread_meta params")?;
    let meta = state.with_store(|s| s.thread_meta(&p.thread_id))?;
    // Never return a bare `null`: the JSON-RPC client carries the result in an
    // Option and reads `ok: null` as "no result" ("daemon returned neither ok
    // nor error"). An unknown thread → an empty object the reader treats as
    // "no metadata", not an error.
    match meta {
        Some(m) => Ok(serde_json::to_value(&m)?),
        None => Ok(serde_json::json!({})),
    }
}

// ── chat read (backfill + optional live stream) ───────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ChatReadParams {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    since: Option<u64>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    tail: bool,
    #[serde(default)]
    live: bool,
}

async fn handle_chat_read<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let p: ChatReadParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let project = p
        .project
        .unwrap_or_else(|| crate::project::resolve(&std::env::current_dir().unwrap_or_default()));
    let since = p.since.unwrap_or(0);
    let offset = p.offset.unwrap_or(0);

    let _ = ensure_subscription(state, &project).await;
    let mut rx = if p.live {
        Some(state.tail_subscribe())
    } else {
        None
    };
    let live_started_at = now_secs();

    let rows = state.with_store(|s| {
        s.list_chat_messages(&project, since, p.limit, offset, p.tail)
            .unwrap_or_default()
    });
    let mut seen: std::collections::HashSet<String> =
        rows.iter().map(|r| r.chat_event_id.clone()).collect();
    let mut cursor = rows
        .iter()
        .map(|r| r.created_at)
        .max()
        .unwrap_or(live_started_at.max(since));

    for row in rows {
        if write_json(writer, &Response::item(id, chat_log_row_to_json(&row)))
            .await
            .is_err()
        {
            let _ = write_json(writer, &Response::end(id)).await;
            return Ok(());
        }
    }

    let Some(ref mut rx) = rx else {
        let _ = write_json(writer, &Response::end(id)).await;
        return Ok(());
    };

    loop {
        match rx.recv().await {
            Ok(TailEvent::Msg {
                project: ev_project,
                thread,
                ..
            }) if ev_project == project && thread.is_none() => {
                let rows = state.with_store(|s| {
                    s.list_chat_messages(&project, cursor, None, 0, false)
                        .unwrap_or_default()
                });
                for row in rows {
                    if !seen.insert(row.chat_event_id.clone()) {
                        continue;
                    }
                    cursor = cursor.max(row.created_at);
                    if write_json(writer, &Response::item(id, chat_log_row_to_json(&row)))
                        .await
                        .is_err()
                    {
                        let _ = write_json(writer, &Response::end(id)).await;
                        return Ok(());
                    }
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
        }
    }
    let _ = write_json(writer, &Response::end(id)).await;
    Ok(())
}

fn chat_log_row_to_json(row: &ChatLogRow) -> serde_json::Value {
    serde_json::json!({
        "event_id": &row.chat_event_id,
        "from_pubkey": &row.from_pubkey,
        "from_slug": &row.from_slug,
        "host": &row.host,
        "project": &row.project,
        "body": &row.body,
        "created_at": row.created_at,
        "from_session": &row.from_session,
        "mentioned_session": &row.mentioned_session,
    })
}

// ── tail (streaming) ──────────────────────────────────────────────────────────

/// Parameters for the `tail` RPC.
#[derive(serde::Deserialize, Default)]
struct TailParams {
    #[serde(default)]
    project: Option<String>,
    /// Number of backfill events (recent messages + roster snapshot), default 20.
    #[serde(default)]
    backfill: Option<u64>,
    /// Return only events after this unix timestamp.
    #[serde(default)]
    since: Option<u64>,
}

async fn handle_tail<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let p: TailParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let project = p.project.clone();
    let backfill_n = p.backfill.unwrap_or(20);
    let since = p.since.unwrap_or(0);

    // Ensure the requested project is in the union subscription.
    if let Some(pr) = &project {
        let _ = ensure_subscription(state, pr).await;
    }

    // Subscribe BEFORE backfill so we don't miss events that arrive during query.
    let mut rx = state.tail_subscribe();

    {
        *state.open_clients.lock().unwrap() += 1;
        state.liveness_changed.notify_waiters();
    }
    let _guard = ClientGuard(state.clone());

    // ── Backfill ────────────────────────────────────────────────────────────
    if backfill_n > 0 {
        let backfill_events = build_backfill(state, project.as_deref(), backfill_n, since);
        for ev in backfill_events {
            if write_json(writer, &Response::item(id, serde_json::to_value(&ev)?))
                .await
                .is_err()
            {
                let _ = write_json(writer, &Response::end(id)).await;
                return Ok(());
            }
        }
    }

    // ── Live stream ─────────────────────────────────────────────────────────
    loop {
        match rx.recv().await {
            Ok(ev) => {
                if tail_event_matches_project(&ev, project.as_deref()) && ev.ts() >= since {
                    if write_json(writer, &Response::item(id, serde_json::to_value(&ev)?))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
        }
    }
    let _ = write_json(writer, &Response::end(id)).await;
    Ok(())
}

/// True when the event belongs to the requested project scope (or no filter).
fn tail_event_matches_project(ev: &TailEvent, project: Option<&str>) -> bool {
    let Some(pr) = project else {
        return true;
    };
    let ev_project = match ev {
        TailEvent::Msg { project, .. } => project.as_str(),
        TailEvent::Sync { project, .. } => project.as_str(),
        TailEvent::Turn { project, .. } => project.as_str(),
        TailEvent::Status { project, .. } => project.as_str(),
        TailEvent::Join { project, .. } => project.as_str(),
        TailEvent::Leave { project, .. } => project.as_str(),
        TailEvent::Sess { project, .. } => project.as_str(),
        TailEvent::Proj { project, .. } => project.as_str(),
        // Profiles are cross-project; always include.
        TailEvent::Profile { .. } => return true,
    };
    ev_project == pr
}

/// Build the backfill event list from the canonical read model.
///
/// Returns recent messages as `Msg` events + a roster snapshot of live sessions
/// as synthetic `Join`/`Turn`/`Status` events, sorted by timestamp ascending.
fn build_backfill(
    state: &Arc<DaemonState>,
    project: Option<&str>,
    limit: u64,
    since: u64,
) -> Vec<TailEvent> {
    let mut events: Vec<TailEvent> = Vec::new();

    // ── Recent messages from the canonical messages table ───────────────────
    let raw_msgs: Vec<(u64, String, String, String, String, Option<String>)> =
        state.with_store(|s| {
            s.recent_messages_for_backfill(project, since, limit)
                .unwrap_or_default()
        });

    for (ts, body, author_pubkey, proj, thread_id, author_session) in raw_msgs {
        // Resolve slug from pubkey.
        let from_slug = state
            .with_store(|s| s.resolve_slug_for_pubkey(&author_pubkey))
            .ok()
            .flatten()
            .unwrap_or_else(|| pubkey_short(&author_pubkey));
        let thread_short = pubkey_short(&thread_id);
        events.push(TailEvent::Msg {
            ts,
            project: proj,
            from: from_slug,
            from_session: author_session,
            to: String::new(), // backfill: recipient not stored inline
            to_session: None,
            thread: Some(thread_short),
            body: body.chars().take(200).collect(),
        });
    }

    // ── Roster snapshot: live sessions ──────────────────────────────────────
    let now = now_secs();
    let since_peer = now.saturating_sub(PRUNE_PEER_AFTER_SECS);

    // Peer sessions as synthetic Join events, status via the SHARED projection.
    let peers = state.with_store(|s| {
        s.peer_session_snapshots(project, since_peer)
            .unwrap_or_default()
    });
    for snap in peers {
        let d = derive_status(&snap, now);
        events.push(TailEvent::Join {
            ts: snap.last_seen,
            project: snap.project.clone(),
            agent: snap.agent_slug.clone(),
            host: snap.host.clone(),
            session: snap.session_id.as_str().to_owned(),
            rel_cwd: snap.rel_cwd.clone(),
        });
        if !d.title.is_empty() || d.busy {
            events.push(TailEvent::Status {
                ts: snap.last_seen,
                project: snap.project.clone(),
                agent: snap.agent_slug.clone(),
                text: d.title.clone(),
                active: d.busy,
            });
        }
    }

    // Own live sessions as synthetic Sess events, busy via the SHARED projection.
    let mine = state.with_store(|s| s.live_session_snapshots(project, 0).unwrap_or_default());
    for snap in mine {
        let d = derive_status(&snap, now);
        events.push(TailEvent::Sess {
            ts: snap.first_seen,
            project: snap.project.clone(),
            agent: snap.agent_slug.clone(),
            session: snap.session_id.as_str().to_owned(),
            state: "start".into(),
            rel_cwd: snap.rel_cwd.clone(),
        });
        if d.busy {
            events.push(TailEvent::Turn {
                ts: snap.turn_started_at,
                project: snap.project.clone(),
                agent: snap.agent_slug.clone(),
                session: snap.session_id.as_str().to_owned(),
                state: "working".into(),
                elapsed_s: None,
            });
        }
    }

    // Sort ascending by timestamp.
    events.sort_by_key(|e| e.ts());
    events
}

// ── relay demux: one subscription, route to all hosted agents ────────────────

fn spawn_demux(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut notifications = state.transport.notifications();
        loop {
            let ev: Option<Event> = match notifications.recv().await {
                Ok(RelayPoolNotification::Event { event, .. }) => Some(*event),
                Ok(RelayPoolNotification::Message {
                    message: RelayMessage::Event { event, .. },
                    ..
                }) => Some(event.into_owned()),
                Ok(_) => None,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(_) => None,
            };
            if let Some(event) = ev {
                handle_incoming(&state, &event);
            }
        }
    });
}

/// Decode one event and apply it. Multi-agent aware: "me" is the SET of hosted
/// local pubkeys; a mention routes by `to_pubkey` to that agent's sessions only.
///
/// Thin dispatch to `provider.materialize` (Phase 5), then derives TailEvents
/// from the domain event using the in-memory tracking maps.
fn handle_incoming(state: &Arc<DaemonState>, event: &Event) {
    let env = crate::fabric::RawEnvelope::Nostr(event.clone());
    let hosted = state.hosted_pubkeys();
    let owners = state.owners.clone();
    let now = now_secs();
    // ALWAYS materialize: store writes are idempotent, and re-deliveries are
    // load-bearing — a refreshed subscription replays stored events, which is
    // how a NEW session receives mentions that predate it.
    let outcome = state.with_store(|s| state.provider.materialize(&env, &hosted, &owners, now, s));
    // The relay pool notifies once PER MATCHING SUBSCRIPTION (scope filters ×
    // live sessions), so the same event reaches here many times. The tail
    // broadcast is NOT idempotent — emit only on first sight of the event id.
    if let Some(de) = outcome.tail {
        if state.first_sight(&event.id.to_hex()) {
            derive_and_emit_tail_events(state, &de, outcome.thread_id.as_deref(), &hosted, now);
        }
    }
    if outcome.wake_mentions {
        crate::tmux::ring_doorbells(state.clone());
    }
}

/// Convert a decoded `DomainEvent` into zero or more `TailEvent`s and emit them.
/// Skip is_self events for presence/status (local lifecycle handled by RPC emitters).
fn derive_and_emit_tail_events(
    state: &Arc<DaemonState>,
    de: &DomainEvent,
    thread_id: Option<&str>,
    hosted: &[String],
    now: u64,
) {
    match de {
        DomainEvent::Proposal(_) => {
            // Proposals are surfaced through the threads read model (the rpc
            // records them as canonical messages); no tail line is derived from
            // the raw inbound event.
        }
        DomainEvent::TurnReply(_) => {
            // A peer's completed turn response (NIP-10 threaded kind:1). Not
            // surfaced on the tail: local turn state is emitted by the RPC
            // lifecycle (Turn working/idle), and peer replies carry no
            // session/turn state we can attribute reliably.
        }
        DomainEvent::Status(s) => {
            // Skip own status — local turn/status is tracked by Turn RPC events.
            if hosted.contains(&s.agent.pubkey) {
                return;
            }
            // The unified per-session Status replaces the old presence heartbeat,
            // so first-sight of a session here is the peer "joined" signal.
            let session_id = s.session_id.as_str().to_owned();
            let is_new = {
                let mut map = state.peer_sessions.lock().unwrap();
                if !map.contains_key(&session_id) {
                    map.insert(
                        session_id.clone(),
                        PeerTracked {
                            first_seen: now,
                            project: s.project.clone(),
                            slug: s.agent.slug.clone(),
                            host: s.host.clone(),
                        },
                    );
                    true
                } else {
                    false
                }
            };
            if is_new {
                state.emit_tail(TailEvent::Join {
                    ts: now,
                    project: s.project.clone(),
                    agent: s.agent.slug.clone(),
                    host: s.host.clone(),
                    session: session_id,
                    rel_cwd: s.rel_cwd.clone(),
                });
            }

            // Dedup per SESSION (not per agent/project): sibling sessions of one
            // agent each track their own (title, busy) transition.
            let key = s.session_id.as_str().to_owned();
            let cur = (s.title.clone(), s.busy);
            let should_emit = {
                let mut map = state.last_status.lock().unwrap();
                if map.get(&key) != Some(&cur) {
                    map.insert(key, cur);
                    true
                } else {
                    false
                }
            };
            if should_emit {
                state.emit_tail(TailEvent::Status {
                    ts: now,
                    project: s.project.clone(),
                    agent: s.agent.slug.clone(),
                    text: s.title.clone(),
                    active: s.busy,
                });
            }
        }

        DomainEvent::Profile(pf) => {
            let is_new = {
                let mut set = state.seen_profiles.lock().unwrap();
                set.insert(pf.agent.pubkey.clone())
            };
            if is_new {
                state.emit_tail(TailEvent::Profile {
                    ts: now,
                    agent: pf.agent.slug.clone(),
                    host: pf.host.clone(),
                    pubkey: pf.agent.pubkey.clone(),
                });
            }
        }

        DomainEvent::Mention(m) => {
            // Only emit for inbound messages (to hosted agents); outbound is
            // handled by rpc_send_message. hosted check ensures we only emit
            // for messages addressed to us.
            if !hosted.contains(&m.to_pubkey) {
                return;
            }
            // Self-authored events never derive a tail line: the publishing RPC
            // already emitted the (slug-resolved) outbound line, and the relay
            // may or may not echo our own events back — suppressing here is the
            // only deterministic way to avoid double-counting.
            if hosted.contains(&m.from.pubkey) {
                return;
            }
            // Exact thread attribution: the materializer reports the canonical
            // thread it filed this message under.
            let thread_short = thread_id.map(pubkey_short);
            // The materializer enriches the slug from the store; if it could
            // not (unknown sender), fall back to the pubkey short code rather
            // than an empty name.
            let from_slug = if m.from.slug.is_empty() {
                pubkey_short(&m.from.pubkey)
            } else {
                m.from.slug.clone()
            };
            state.emit_tail(TailEvent::Msg {
                ts: now,
                project: m.project.clone(),
                from: from_slug,
                from_session: m.from_session.as_ref().map(|s| s.as_str().to_owned()),
                to: pubkey_short(&m.to_pubkey),
                to_session: m.target_session.as_ref().map(|s| s.as_str().to_owned()),
                thread: thread_short,
                body: m.body.chars().take(200).collect(),
            });
        }

        DomainEvent::ChatMessage(chat) => {
            // Local publishes emit their own outbound tail line in rpc_chat_write.
            if hosted.contains(&chat.from.pubkey) {
                return;
            }
            let from_slug = if chat.from.slug.is_empty() {
                pubkey_short(&chat.from.pubkey)
            } else {
                chat.from.slug.clone()
            };
            let to = chat
                .mentioned_pubkey
                .as_deref()
                .map(pubkey_short)
                .unwrap_or_else(|| "project-chat".to_string());
            state.emit_tail(TailEvent::Msg {
                ts: now,
                project: chat.project.clone(),
                from: from_slug,
                from_session: chat.from_session.as_ref().map(|s| s.as_str().to_owned()),
                to,
                to_session: chat
                    .mentioned_session
                    .as_ref()
                    .map(|s| s.as_str().to_owned()),
                thread: None,
                body: chat.body.chars().take(200).collect(),
            });
        }

        DomainEvent::Activity(_) => {
            // Activity events are not emitted on the tail (they're durable
            // narrative, not real-time transitions).
        }
    }
}

// ── startup fetch of stored mentions (offline delivery) ──────────────────────

async fn fetch_mentions_into_inbox(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
) -> Result<()> {
    let owners = state.owners.clone();
    let wake_count = state.provider.catch_up_mentions(rec, &owners).await?;
    if wake_count > 0 {
        crate::tmux::ring_doorbells(state.clone());
    }
    Ok(())
}

// ── pruner ───────────────────────────────────────────────────────────────────

fn spawn_pruner(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let now = now_secs();
            let before = now.saturating_sub(PRUNE_PEER_AFTER_SECS);

            // Identify which peer sessions will be pruned by checking the map
            // against sessions that are about to expire.
            let expired_sessions: Vec<String> = {
                let map = state.peer_sessions.lock().unwrap();
                // We'll emit Leave for sessions in our map whose last_seen is
                // older than `before`. We need to cross-reference with the store.
                map.keys().cloned().collect()
            };

            // Query which of those are actually expired in the store.
            let still_alive: std::collections::HashSet<String> = state
                .with_store(|s| s.list_peer_sessions(None, before).unwrap_or_default())
                .into_iter()
                .map(|p| p.session_id)
                .collect();

            // Prune from DB.
            state.with_store(|s| {
                let _ = s.prune_peer_sessions(before);
            });

            // Emit Leave for sessions that were in our map but are now expired.
            let to_leave: Vec<(String, PeerTracked)> = {
                let mut map = state.peer_sessions.lock().unwrap();
                let expired: Vec<String> = expired_sessions
                    .into_iter()
                    .filter(|sid| !still_alive.contains(sid))
                    .collect();
                let mut leaves = Vec::new();
                for sid in expired {
                    if let Some(tracked) = map.remove(&sid) {
                        leaves.push((sid, tracked));
                    }
                }
                leaves
            };
            for (sid, tracked) in to_leave {
                let online_s = now.saturating_sub(tracked.first_seen);
                state.emit_tail(TailEvent::Leave {
                    ts: now,
                    project: tracked.project,
                    agent: tracked.slug,
                    host: tracked.host,
                    session: sid,
                    online_s,
                });
            }
        }
    });
}

// ── idle-exit watcher ─────────────────────────────────────────────────────────

fn spawn_idle_watcher(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        loop {
            state.liveness_changed.notified().await;
            if is_idle(&state) {
                tokio::select! {
                    _ = tokio::time::sleep(grace()) => {
                        if is_idle(&state) {
                            eprintln!("[daemon] idle for grace period; exiting");
                            state.shutdown.notify_waiters();
                            return;
                        }
                    }
                    _ = state.liveness_changed.notified() => {}
                }
            }
        }
    });
}

fn is_idle(state: &Arc<DaemonState>) -> bool {
    *state.open_clients.lock().unwrap() == 0 && state.live_session_count() == 0
}

// ── status-outbox drainer ──────────────────────────────────────────────────────

/// Build the wire `Status` for one snapshot, re-arming the NIP-40 expiration to
/// `now + STATUS_TTL_SECS`. Runs the SHARED `derive_status` projection so an idle
/// session publishes a blanked activity (only the persistent title survives).
fn status_from_snapshot(
    snap: &SessionSnapshot,
    now: u64,
    thread_root_id: Option<String>,
) -> crate::domain::Status {
    let d = derive_status(snap, now);
    crate::domain::Status {
        agent: crate::domain::AgentRef::new(snap.agent_pubkey.clone(), snap.agent_slug.clone()),
        project: snap.project.clone(),
        session_id: snap.session_id.clone(),
        host: snap.host.clone(),
        title: snap.title.clone(),
        activity: d.activity,
        busy: d.busy,
        rel_cwd: snap.rel_cwd.clone(),
        expires_at: Some(now + crate::domain::STATUS_TTL_SECS),
        thread_root_id,
    }
}

/// Look up a session's conversation thread root for the kind:30315 link, or
/// `None` before the first prompt has been recorded.
fn thread_root_for(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    state.with_store(|s| {
        let (root, _) = s.get_thread_event_ids(session_id);
        (!root.is_empty()).then_some(root)
    })
}

/// Heartbeat re-arm: every `HEARTBEAT_SECS`, re-publish the current kind:30315 for
/// every live locally-hosted session so its NIP-40 `expiration` is pushed forward
/// to `now + STATUS_TTL_SECS`. The outbox only fires on state CHANGES; a live-but-
/// idle session produces none, so without this its relay event would expire after
/// `STATUS_TTL_SECS` and read as gone despite the runtime heartbeating `last_seen`
/// locally. This is the piece that turns store-side freshness into relay liveness.
fn spawn_status_heartbeat_publisher(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(crate::domain::HEARTBEAT_SECS));
        loop {
            tick.tick().await;
            let now = now_secs();
            let fresh_since = now.saturating_sub(crate::domain::STATUS_TTL_SECS);
            let snaps =
                state.with_store(|s| s.all_live_local_snapshots(fresh_since).unwrap_or_default());
            for snap in snaps {
                // Only locally-hosted agents have signing keys.
                let keys = match state.keys_for(&snap.agent_pubkey) {
                    Some(k) => k,
                    None => continue,
                };
                let root = thread_root_for(&state, snap.session_id.as_str());
                let status = status_from_snapshot(&snap, now, root);
                let _ = state.provider.set_status(&status, &keys).await;
            }
        }
    });
}

/// Drain the `status_outbox`: publish each pending kind:30315 via the provider's
/// `set_status`, recording the native event id (or a retryable failure). Woken
/// instantly by `status_outbox_notify` on every transition, and polled every 2s
/// as a fallback for transitions enqueued by the runtime (distill/seed/heartbeat).
fn spawn_status_outbox_drainer(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        loop {
            // Drain the backlog while we keep making progress (so a startup burst
            // clears fast); stop if a whole batch failed to avoid a tight spin.
            loop {
                let items = state.with_store(|s| s.pending_status_outbox(32).unwrap_or_default());
                if items.is_empty() {
                    break;
                }
                let mut progressed = false;
                for item in items {
                    let now = now_secs();
                    // Only locally-hosted agents have signing keys; a row for an
                    // unhosted agent can never publish — record and skip it.
                    let keys = match state.keys_for(&item.snapshot.agent_pubkey) {
                        Some(k) => k,
                        None => {
                            state.with_store(|s| {
                                s.mark_status_failed(
                                    &item.session_id,
                                    item.state_version,
                                    "no signing keys for agent",
                                )
                                .ok();
                            });
                            continue;
                        }
                    };
                    let root = thread_root_for(&state, item.snapshot.session_id.as_str());
                    let status = status_from_snapshot(&item.snapshot, now, root);
                    match state.provider.set_status(&status, &keys).await {
                        Ok(eid) => {
                            state.with_store(|s| {
                                s.mark_status_published(
                                    &item.session_id,
                                    item.state_version,
                                    &eid.to_hex(),
                                )
                                .ok();
                            });
                            progressed = true;
                        }
                        Err(e) => {
                            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                                eprintln!(
                                    "[daemon] status publish failed for {}: {e:#}",
                                    item.session_id
                                );
                            }
                            state.with_store(|s| {
                                s.mark_status_failed(
                                    &item.session_id,
                                    item.state_version,
                                    &format!("{e:#}"),
                                )
                                .ok();
                            });
                        }
                    }
                }
                if !progressed {
                    break;
                }
            }
            tokio::select! {
                _ = state.status_outbox_notify.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    });
}

// ── session lifecycle ─────────────────────────────────────────────────────────

async fn spawn_session(state: &Arc<DaemonState>, params: EngineParams) -> Result<()> {
    let session_id = params.session_id.clone();
    let pubkey = params.agent_pubkey.clone();
    let project = params.project.clone();

    state.hosted.lock().unwrap().insert(
        pubkey.clone(),
        HostedAgent {
            keys: params.keys.clone(),
        },
    );
    ensure_subscription(state, &project).await?;

    let cancel = Arc::new(Notify::new());
    state.sessions.lock().unwrap().insert(
        session_id.clone(),
        SessionHandle {
            cancel: cancel.clone(),
        },
    );
    state.liveness_changed.notify_waiters();

    let st = state.clone();
    let sid = session_id.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(params, provider, store, cancel).await;
        if let Err(e) = res {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] session {sid} task error: {e:#}");
            }
        }
        st.sessions.lock().unwrap().remove(&sid);
        prune_hosted(&st);
        st.liveness_changed.notify_waiters();
    });
    Ok(())
}

fn prune_hosted(state: &Arc<DaemonState>) {
    let live: std::collections::HashSet<String> = state
        .with_store(|s| s.list_alive_sessions().unwrap_or_default())
        .into_iter()
        .map(|r| r.agent_pubkey)
        .collect();
    state
        .hosted
        .lock()
        .unwrap()
        .retain(|pk, _| live.contains(pk));
}

fn cancel_session(state: &Arc<DaemonState>, session_id: &str) -> bool {
    if let Some(h) = state.sessions.lock().unwrap().get(session_id) {
        h.cancel.notify_waiters();
        true
    } else {
        false
    }
}

async fn ensure_subscription(state: &Arc<DaemonState>, project: &str) -> Result<()> {
    {
        let mut projs = state.subscribed_projects.lock().unwrap();
        if !projs.iter().any(|p| p == project) {
            projs.push(project.to_string());
        }
    }
    resubscribe(state).await
}

/// Rebuild and apply the union subscription across all hosted agents/projects.
async fn resubscribe(state: &Arc<DaemonState>) -> Result<()> {
    let mut authors: Vec<String> = state.hosted_pubkeys();
    authors.sort();
    authors.dedup();

    let projects = state.subscribed_projects.lock().unwrap().clone();
    let owners = state.owners.clone();
    let hosted = state.hosted_pubkeys();

    for project in &projects {
        if hosted.is_empty() {
            let scope = crate::fabric::Scope {
                authors: authors.clone(),
                project: Some(project.clone()),
                mentions_to: None,
                owners: owners.clone(),
                thread: None,
            };
            state.provider.subscribe(scope).await?;
        } else {
            for me in &hosted {
                let scope = crate::fabric::Scope {
                    authors: authors.clone(),
                    project: Some(project.clone()),
                    mentions_to: Some(me.clone()),
                    owners: owners.clone(),
                    thread: None,
                };
                state.provider.subscribe(scope).await?;
            }
        }
    }
    Ok(())
}

/// Revive sessions a previous daemon left behind (skew re-exec / crash),
/// rebuilding from the canonical `session_state` aggregate. For each ACTIVE
/// session: respawn the engine task if its watched pid is still alive, else end
/// the canonical session AND mark the runtime row dead (so `who`/presence don't
/// lie after a restart). `watch_pid` lives in the kept `sessions` runtime table
/// (session_state carries no pid), so it is joined per session.
async fn reconcile_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let snaps = state.with_store(|s| s.live_session_snapshots(None, 0).unwrap_or_default());
    for snap in snaps {
        let session_id = snap.session_id.as_str().to_owned();
        let watch_pid = state
            .with_store(|s| s.get_session(&session_id).ok().flatten())
            .and_then(|r| r.watch_pid);
        let pid_ok = watch_pid.map(pid_alive).unwrap_or(false);
        if !pid_ok {
            state.with_store(|s| {
                s.end_session(&session_id, now).ok();
                s.mark_session_dead(&session_id).ok();
            });
            continue;
        }
        let id = match identity::load_or_create(&config::edge_home(), &snap.agent_slug, now) {
            Ok(i) => i,
            Err(_) => continue,
        };
        // Re-establish ownership/membership + the group-state subscription for
        // revived sessions. Idempotent: the owned_groups/group_members cache
        // persists across restarts, so already-owned groups skip republishing.
        state
            .provider
            .open_project(&snap.project, &id.pubkey_hex())
            .await;
        if let Err(e) = ensure_subscription(state, &snap.project).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!(
                    "[daemon] ensure_subscription({}) failed: {e:#}",
                    snap.project
                );
            }
        }
        let ep = engine_params_for(
            &state.cfg,
            &id,
            &snap.agent_slug,
            &session_id,
            &snap.project,
            &snap.rel_cwd,
            watch_pid,
        );
        let _ = spawn_session(state, ep).await;
    }
    // Any registration/end transitions above enqueued publishes.
    state.status_outbox_notify.notify_waiters();
}

fn engine_params_for(
    cfg: &Config,
    id: &AgentIdentity,
    agent_slug: &str,
    session_id: &str,
    project: &str,
    rel_cwd: &str,
    watch_pid: Option<i32>,
) -> EngineParams {
    EngineParams {
        agent_slug: agent_slug.to_string(),
        agent_pubkey: id.pubkey_hex(),
        keys: id.keys.clone(),
        project: project.to_string(),
        session_id: session_id.to_string(),
        host: cfg.host.clone(),
        rel_cwd: rel_cwd.to_string(),
        owners: cfg.whitelisted_pubkeys.clone(),
        relays: cfg.relays.clone(),
        watch_pid,
        store_path: store_path(),
        heartbeat: env_duration("TENEX_EDGE_HEARTBEAT_MS", Duration::from_secs(30)),
        obs_interval: env_duration("TENEX_EDGE_OBS_MS", Duration::from_secs(5)),
        status_ttl: Duration::from_secs(env_u64("TENEX_EDGE_STATUS_TTL_S", 90)),
        turn_first: Duration::from_secs(env_u64("TENEX_EDGE_TURN_FIRST_S", 30)),
        // 0 = disabled: the title re-distills on each new user message, so an
        // in-turn safety re-distill is opt-in (set TENEX_EDGE_TURN_REPEAT_S > 0).
        turn_repeat: Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 0)),
    }
}

fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

// ── small helpers ─────────────────────────────────────────────────────────────

/// Cap on the first-sight event-id memory (events, not bytes). Relay
/// re-notifications arrive within milliseconds of each other, so even a small
/// window suffices; 4096 also absorbs startup catch-up bursts.
const SEEN_EVENTS_CAP: usize = 4096;

impl DaemonState {
    /// True exactly once per native event id (bounded memory). Subsequent
    /// sightings — the relay pool notifying for every matching subscription —
    /// return false and must be ignored.
    fn first_sight(&self, event_id: &str) -> bool {
        let mut g = self.seen_events.lock().unwrap();
        let (set, order) = &mut *g;
        if set.contains(event_id) {
            return false;
        }
        set.insert(event_id.to_owned());
        order.push_back(event_id.to_owned());
        if order.len() > SEEN_EVENTS_CAP {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
        }
        true
    }

    fn tail_subscribe(&self) -> tokio::sync::broadcast::Receiver<TailEvent> {
        self.tail_tx.subscribe()
    }
    fn emit_tail(&self, ev: TailEvent) {
        let _ = self.tail_tx.send(ev);
    }
}

/// Serialize inbox rows for the CLI, which renders the email-like envelope via
/// `cli::format_envelope`. `self_host` is the daemon's own host, so the client
/// can decide whether the sender is `[remote: …]` without re-deriving it. `id`
/// is the short prefix the receiver passes to `inbox reply --id`.
fn rows_to_json(rows: &[InboxRow], self_host: &str) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|r| {
            serde_json::json!({
                "from_slug": r.from_slug,
                "project": r.project,
                "from_session": r.from_session,
                "host": r.host,
                "self_host": self_host,
                "subject": r.subject,
                "branch": r.branch,
                "commit": r.commit,
                "dirty": r.dirty,
                "created_at": r.created_at,
                "id": crate::cli::mention_short_id(&r.mention_event_id),
                "mention_event_id": r.mention_event_id,
                "body": r.body,
            })
        })
        .collect()
}

fn chat_rows_to_json(rows: &[ChatInboxRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|r| {
            serde_json::json!({
                "from_slug": r.from_slug,
                "project": r.project,
                "from_session": r.from_session,
                "host": "",
                "subject": "",
                "created_at": r.created_at,
                "id": crate::cli::mention_short_id(&r.chat_event_id),
                "mention_event_id": r.chat_event_id,
                "body": r.body,
            })
        })
        .collect()
}

fn sort_message_json(rows: &mut [serde_json::Value]) {
    rows.sort_by_key(|row| row["created_at"].as_i64().unwrap_or_default());
}

fn env_duration(key: &str, default: Duration) -> Duration {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}
fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
