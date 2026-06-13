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
use crate::domain::{DomainEvent, Mention};
use crate::fabric::provider::Kind1Nip29Provider;
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, route_mention_into_with_id, EngineParams};
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, pubkey_short, session_short_code, SessionId};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
    mention_notify: Notify,
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
    seen_events: Mutex<(std::collections::HashSet<String>, std::collections::VecDeque<String>)>,
    /// Pubkeys for which a Profile event has already been emitted, for first-seen dedup.
    seen_profiles: Mutex<std::collections::HashSet<String>>,
    /// Last-seen status text per (pubkey, project) for dedup.
    last_status: Mutex<HashMap<(String, String), String>>,
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
        self.hosted.lock().unwrap().get(pubkey).map(|h| h.keys.clone())
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

    let store = Arc::new(Mutex::new(Store::open(&store_path())?));
    let provider = Arc::new(Kind1Nip29Provider::new(
        transport.clone(),
        store.clone(),
        cfg.user_nsec.clone(),
        cfg.whitelisted_pubkeys.clone(),
        &cfg.relays,
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
        mention_notify: Notify::new(),
        tail_tx: tokio::sync::broadcast::channel(512).0,
        open_clients: Mutex::new(0),
        liveness_changed: Notify::new(),
        shutdown: Notify::new(),
        peer_sessions: Mutex::new(HashMap::new()),
        seen_events: Mutex::new((std::collections::HashSet::new(), std::collections::VecDeque::new())),
        seen_profiles: Mutex::new(std::collections::HashSet::new()),
        last_status: Mutex::new(HashMap::new()),
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

    reconcile_sessions(&state).await;
    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    spawn_idle_watcher(state.clone());

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
            "wait_for_mention" => {
                let resp = handle_wait_for_mention(&state, &req).await;
                write_json(&mut writer, &resp).await?;
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

// ── dispatch (one-shot verbs) ────────────────────────────────────────────────

async fn dispatch(state: &Arc<DaemonState>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "ping" => Ok(serde_json::json!({"pong": true})),
        "who" => rpc_who(state, &req.params),
        "session_start" => rpc_session_start(state, &req.params).await,
        "session_end" => rpc_session_end(state, &req.params),
        "send_message" => rpc_send_message(state, &req.params).await,
        "propose" => rpc_propose(state, &req.params).await,
        "inbox" => rpc_inbox(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "acl" => rpc_acl(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "user_prompt" => rpc_user_prompt(state, &req.params).await,
        "project_list" => rpc_project_list(state).await,
        "project_edit" => rpc_project_edit(state, &req.params).await,
        "project_add" => rpc_project_add(state, &req.params).await,
        "inbox_reply" => rpc_inbox_reply(state, &req.params).await,
        "statusline" => rpc_statusline(state, &req.params),
        "list_threads" => rpc_list_threads(state, &req.params).await,
        "messages" => rpc_messages(state, &req.params),
        "thread_meta" => rpc_thread_meta(state, &req.params),
        "tmux_status" => tmux_rpc::rpc_tmux_status(state),
        "tmux_send" => tmux_rpc::rpc_tmux_send(state, &req.params).await,
        "tmux_spawn" => tmux_rpc::rpc_tmux_spawn(state, &req.params).await,
        "tmux_attach" => tmux_rpc::rpc_tmux_attach(state, &req.params),
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
        if let Some(rec) = state
            .with_store(|s| s.latest_alive_session_for_agent_in_project(agent, &project))?
        {
            return Ok(rec);
        }
        anyhow::bail!(
            "no active tenex-edge session for agent {agent:?} in project {project:?} (run session-start, or pass --session)"
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
    let snapshot = state
        .with_store(|s| crate::cli::load_who_snapshot(s, current_project.as_deref(), p.all, now, &host))?;
    Ok(serde_json::to_value(snapshot)?)
}

// ── session_start / session_end ──────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SessionStartParams {
    agent: String,
    #[serde(default)]
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
}

async fn rpc_session_start(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionStartParams =
        serde_json::from_value(params.clone()).context("parsing session_start params")?;
    let edge = config::edge_home();
    config::ensure_dir(&edge)?;
    let id = identity::load_or_create(&edge, &p.agent, now_secs())?;
    let _ = crate::acl::allow(&id.pubkey_hex(), &p.agent); // own fleet auto-trusted
    let cwd = p
        .cwd
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let project = crate::project::resolve(&cwd);
    let rel_cwd = crate::project::rel_cwd(&cwd);
    let session_id = p.session_id.unwrap_or_else(gen_session_id);

    // A new session arriving on the SAME watched pid (same agent/project/host)
    // means the harness restarted/cleared without a session-end: kill the stale
    // sibling so `who` doesn't show ghosts.
    if let Some(watch_pid) = p.watch_pid {
        let stale_ids: Vec<String> = state
            .with_store(|s| s.list_alive_sessions().unwrap_or_default())
            .into_iter()
            .filter(|rec| {
                rec.session_id != session_id
                    && rec.agent_slug == p.agent
                    && rec.project == project
                    && rec.host == state.host
                    && rec.watch_pid == Some(watch_pid)
            })
            .map(|rec| rec.session_id)
            .collect();
        for old_id in stale_ids {
            cancel_session(state, &old_id);
            state.with_store(|s| {
                s.mark_session_dead(&old_id).ok();
            });
        }
    }

    state.with_store(|s| {
        s.upsert_session(&crate::state::SessionRecord {
            session_id: session_id.clone(),
            agent_slug: p.agent.clone(),
            agent_pubkey: id.pubkey_hex(),
            project: project.clone(),
            host: state.host.clone(),
            child_pid: None,
            watch_pid: p.watch_pid,
            created_at: now_secs(),
            alive: true,
            rel_cwd: rel_cwd.clone(),
        })
        .ok();
        s.touch_session(&session_id, now_secs()).ok();
        // Record the absolute path for this project so the tmux spawn command
        // can cd to it.
        s.upsert_project_path(&project, &cwd.to_string_lossy(), now_secs()).ok();
        // Register the tmux endpoint if the hook env supplied TMUX_PANE.
        if let Some(ref pane) = p.tmux_pane {
            if !pane.is_empty() {
                let meta = serde_json::json!({
                    "socket": p.tmux_socket.as_deref().unwrap_or(""),
                    "pane_command": p.agent,
                })
                .to_string();
                s.upsert_session_endpoint(&session_id, "tmux", pane, &meta, now_secs()).ok();
            }
        }
    });

    // Idempotent re-start (session reassert): the engine task already runs.
    if state.sessions.lock().unwrap().contains_key(&session_id) {
        return Ok(serde_json::json!({ "session_id": session_id }));
    }

    // Make sure the project's NIP-29 group exists and this agent is a member
    // BEFORE the engine starts publishing, so its presence lands in a group it
    // already belongs to. Best-effort: never block a session from starting.
    state.provider.open_project(&project, &id.pubkey_hex()).await;
    // Keep the relay-authored group state (39000/39001/39002) subscribed so the
    // membership cache stays current — "check which groups we own at all times".
    if let Err(e) = ensure_subscription(state, &project).await {
        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
            eprintln!("[daemon] ensure_subscription({project}) failed: {e:#}");
        }
    }

    let ep = engine_params_for(&state.cfg, &id, &p.agent, &session_id, &project, &rel_cwd, p.watch_pid);
    spawn_session(state, ep).await?;

    state.emit_tail(TailEvent::Sess {
        ts: now_secs(),
        project: project.clone(),
        agent: p.agent.clone(),
        session: session_id.clone(),
        state: "start".into(),
        rel_cwd: rel_cwd.clone(),
    });

    // If this pane was created via `tmux_spawn` (registered as a pending spawn),
    // inject the first prompt into it once the harness is ready rather than
    // waiting for `ring_doorbells` to fire a generic nudge. The entry is only
    // present when `spawn_agent` created this window; ordinary harness starts
    // (and repeated `session_start` reasserts) never match. Consuming here
    // ensures injection fires exactly once regardless of call count.
    let pending_spawn = p
        .tmux_pane
        .as_deref()
        .filter(|pane| !pane.is_empty())
        .and_then(crate::tmux::consume_pending_spawn);

    // If the spawn carried a triggering mention, write it to this session's
    // inbox NOW — before the harness runs `tenex-edge inbox` — so the agent
    // finds the message on its very first turn.
    if let Some(ref ps) = pending_spawn {
        if let Some(ref m) = ps.mention {
            state.with_store(|s| {
                s.enqueue_mention(&crate::state::InboxRow {
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
                })
                .ok()
            });
            state.mention_notify.notify_waiters();
        }
    }

    if let (Some(ps), Some(pane)) = (pending_spawn, p.tmux_pane.clone()) {
        let prompt = ps.prompt.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::tmux::inject_spawn_prompt(&pane, &prompt).await {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!("[tmux] spawn prompt inject failed for pane {pane}: {e:#}");
                }
            } else if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[tmux] spawn prompt {prompt:?} injected into pane {pane}");
            }
        });
    }

    Ok(serde_json::json!({ "session_id": session_id }))
}


#[derive(serde::Deserialize)]
struct SessionEndParams {
    session: String,
}

fn rpc_session_end(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let rec = state.with_store(|s| s.get_session(&p.session).ok().flatten());
    let existed = rec.is_some();
    if let Some(ref rec) = rec {
        cancel_session(state, &p.session);
        state.with_store(|s| {
            s.mark_session_dead(&p.session).ok();
        });
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

fn gen_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("te-{nanos:x}-{}", std::process::id())
}

// ── send_message ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct SendMessageParams {
    recipient: String,
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

    let recipient = state.with_store(|s| resolve_recipient(s, &rec.project, &p.recipient))?;

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
    let is_local = state.hosted_pubkeys().iter().any(|h| h == &recipient.pubkey);
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

    if is_local {
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
            state.mention_notify.notify_waiters();
            crate::tmux::ring_doorbells(state.clone());
        }
    }

    // SPAWN-ON-SEND: when the recipient is addressed as `slug@project` (no
    // specific target session), and the recipient is one of OUR locally-owned
    // agents, spawn a fresh tmux window for it so the message is actually seen.
    // Gated on `get_local_agent_slug_by_pubkey` (the "is locally owned?"
    // predicate) so we never try to spawn a remote agent.
    if recipient.target_session.is_none() {
        let to_pk = recipient.pubkey.clone();
        let project2 = recipient.project.clone();
        let slug_opt = state.with_store(|s| s.get_local_agent_slug_by_pubkey(&to_pk));
        if let Some(slug) = slug_opt {
            let state2 = Arc::clone(state);
            // Capture the triggering mention so the spawned session's inbox is
            // pre-loaded before the harness receives its first prompt.
            let pending_mention = crate::tmux::PendingMention {
                event_id: receipt.native_event_id.clone(),
                from_pubkey: id.pubkey_hex(),
                from_slug: rec.agent_slug.clone(),
                from_session: rec.session_id.clone(),
                project: recipient.project.clone(),
                body: body.clone(),
                created_at: now_secs(),
            };
            tokio::spawn(async move {
                match crate::tmux::spawn_agent(&state2, &slug, &project2).await {
                    Ok(pane_id) => {
                        crate::tmux::register_pending_spawn_with_mention(
                            &pane_id,
                            pending_mention,
                        );
                    }
                    Err(e) => {
                        if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                            eprintln!("[tmux] spawn failed for {slug}@{project2}: {e:#}");
                        }
                    }
                }
            });
        }
    }

    Ok(serde_json::json!({ "to_pubkey": recipient.pubkey, "target_session": recipient.target_session }))
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
    let event_id = state
        .provider
        .publish(&ev, &id.keys)
        .await
        .context("publishing proposal")?;
    let eid_hex = event_id.to_hex();

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
    }))
}

struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

fn resolve_recipient(
    store: &Store,
    my_project: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    if let Some((slug, proj)) = target.split_once('@') {
        let pk = store
            .resolve_agent_pubkey(slug, Some(proj))?
            .with_context(|| format!("can't resolve {slug}@{proj} (no presence/profile seen yet)"))?;
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
        // Try matching against hash-based session short codes (from `who` display).
        // This is a fallback for when users copy session codes from `who` output.
        if let Some(found) = find_session_by_hash(store, target)? {
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

/// Try to find a session (peer or own) matching the given hash code.
/// Hash codes are what `who` displays for sessions (6-char hex strings).
fn find_session_by_hash(store: &Store, hash_code: &str) -> Result<Option<SessionMatch>> {
    let target_code = hash_code.to_lowercase();

    // Search peer sessions
    if let Ok(peers) = store.list_peer_sessions(None, 0) {
        for peer in peers {
            if session_short_code(&peer.session_id).to_lowercase() == target_code {
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
            if session_short_code(&session.session_id).to_lowercase() == target_code {
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

async fn rpc_inbox(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: InboxParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(state, p.session.as_deref(), p.env_session.as_deref(), p.cwd.as_deref(), p.agent.as_deref())?;
    let _ = fetch_mentions_into_inbox(state, &rec).await;

    let rows = state.with_store(|s| {
        let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
        for r in &rows {
            s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                .ok();
        }
        rows
    });
    let pending = state.with_store(|s| s.list_pending_agents().unwrap_or_default());
    let rows_json = rows_to_json(&rows, &state.host);

    Ok(serde_json::json!({
        "rows": rows_json,
        "pending_agents": pending.iter().map(|p| serde_json::json!({"slug": p.slug, "pubkey": p.pubkey})).collect::<Vec<_>>(),
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

    let prev_started = state.with_store(|s| {
        let (_, prev) = s.get_turn_state(&p.session).unwrap_or((false, 0));
        s.mark_turn_start(&p.session, now_secs()).ok();
        if let Some(path) = p.transcript.as_deref().filter(|x| !x.is_empty()) {
            s.set_session_transcript(&p.session, path).ok();
            // Snapshot the last assistant text so rpc_turn_end can poll until a
            // *new* (different) response appears — Claude Code writes the
            // transcript after the stop hook fires, so reading at stop time often
            // returns the previous turn's content.
            let baseline = crate::transcript::read_last_assistant_text(std::path::Path::new(path))
                .unwrap_or_default();
            s.set_last_assistant_text_at_turn_start(&p.session, &baseline).ok();
        }
        prev
    });

    let rec = match state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
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

fn rpc_turn_check(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: TurnCheckParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let rec = resolve_session(state, p.session.as_deref(), p.env_session.as_deref(), p.cwd.as_deref(), p.agent.as_deref())?;
    let context = crate::cli::assemble_turn_check_context(&state.store, &rec.session_id, &state.host)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize)]
struct TurnEndParams {
    session: String,
}

async fn rpc_turn_end(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: TurnEndParams =
        serde_json::from_value(params.clone()).context("parsing turn_end params")?;
    if !p.session.is_empty() {
        // Read turn_started_at BEFORE marking end, so we can compute elapsed.
        // Thread IDs are captured NOW so a concurrent user_prompt for the next
        // turn cannot overwrite last_prompt_event_id before we publish.
        let (was_working, turn_started_at) = state
            .with_store(|s| s.get_turn_state(&p.session).unwrap_or((false, 0)));
        let (root_event_id, last_prompt_event_id, transcript_path, baseline_text) =
            state.with_store(|s| {
                s.mark_turn_end(&p.session).ok();
                let (root, prompt) = s.get_thread_event_ids(&p.session);
                let transcript = s.get_session_transcript(&p.session).ok().flatten();
                let baseline = s.get_last_assistant_text_at_turn_start(&p.session);
                (root, prompt, transcript, baseline)
            });

        // Publish the NIP-10 TurnReply when we have full threading context.
        if !root_event_id.is_empty() && !last_prompt_event_id.is_empty() {
            if let Some(rec) = state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
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
            if let Some(rec) = state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
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

// ── acl ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct AclParams {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    target: Option<String>,
}

async fn rpc_acl(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: AclParams = serde_json::from_value(params.clone()).unwrap_or_default();
    match p.action.as_deref() {
        Some("allow") => {
            let target = p.target.context("acl allow needs a target")?;
            let (pk, slug) = state.with_store(|s| resolve_acl_target(s, &target))?;
            let host = state.with_store(|s| {
                s.list_pending_agents().ok()
                    .and_then(|v| v.into_iter().find(|a| a.pubkey == pk))
                    .map(|a| a.host)
                    .unwrap_or_default()
            });
            crate::acl::allow(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
            });
            // Newly-trusted author: refresh the union subscription.
            resubscribe(state).await.ok();
            state.emit_tail(TailEvent::Acl {
                ts: now_secs(),
                action: "admitted".into(),
                agent: slug.clone(),
                host,
                pubkey: pk.clone(),
                role: None,
            });
            Ok(serde_json::json!({ "slug": slug, "pubkey": pk }))
        }
        Some("block") => {
            let target = p.target.context("acl block needs a target")?;
            let (pk, slug) = state.with_store(|s| resolve_acl_target(s, &target))?;
            let host = state.with_store(|s| {
                s.list_pending_agents().ok()
                    .and_then(|v| v.into_iter().find(|a| a.pubkey == pk))
                    .map(|a| a.host)
                    .unwrap_or_default()
            });
            crate::acl::block(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
            });
            state.emit_tail(TailEvent::Acl {
                ts: now_secs(),
                action: "blocked".into(),
                agent: slug.clone(),
                host,
                pubkey: pk.clone(),
                role: None,
            });
            Ok(serde_json::json!({ "slug": slug, "pubkey": pk }))
        }
        _ => {
            let pending = state.with_store(|s| s.list_pending_agents().unwrap_or_default());
            let allowed = crate::acl::allowed().len();
            let blocked = crate::acl::blocked().len();
            Ok(serde_json::json!({
                "pending": pending.iter().map(|p| serde_json::json!({"slug": p.slug, "pubkey": p.pubkey, "host": p.host})).collect::<Vec<_>>(),
                "allowed": allowed,
                "blocked": blocked,
            }))
        }
    }
}

fn resolve_acl_target(store: &Store, target: &str) -> Result<(String, String)> {
    if target.len() == 64 && target.chars().all(|c| c.is_ascii_hexdigit()) {
        let slug = store
            .list_pending_agents()?
            .into_iter()
            .find(|p| p.pubkey == target)
            .map(|p| p.slug)
            .unwrap_or_else(|| "agent".to_string());
        return Ok((target.to_string(), slug));
    }
    let m = store
        .list_pending_agents()?
        .into_iter()
        .find(|p| p.slug == target);
    match m {
        Some(p) => Ok((p.pubkey, p.slug)),
        None => anyhow::bail!("no pending agent named {target:?}; use a pubkey or `tenex-edge acl list`"),
    }
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
async fn rpc_user_prompt(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
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

    let rec = resolve_session(state, p.session.as_deref(), p.env_session.as_deref(), p.cwd.as_deref(), p.agent.as_deref())?;
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
    let event_id = state.provider.publish(&ev, &user_keys).await?;

    // NIP-10 thread tracking: first prompt becomes the root; every prompt is
    // the "last trigger" the next TurnReply will reply to.
    let eid = event_id.to_hex();
    let sid = rec.session_id.clone();
    state.with_store(|s| {
        let (root, _) = s.get_thread_event_ids(&sid);
        let new_root = if root.is_empty() { eid.clone() } else { root };
        s.set_thread_event_ids(&sid, &new_root, &eid).ok();
    });

    // Suppress the relay echo of our own prompt: this RPC is only ever invoked
    // by the LOCAL harness's user-prompt-submit hook, so the agent already has
    // the prompt in front of it. Routing the echoed kind:1 back into this same
    // agent's inbox would create a phantom unread mention — and because the tmux
    // doorbell auto-submits its nudge text as a prompt, that echo perpetually
    // re-arms the doorbell (an infinite "you have new mentions" loop). Marking
    // the event seen makes `route_mention_into` drop the untargeted echo.
    // Remote prompts never pass through this RPC, so they are unaffected.
    state.with_store(|s| s.mark_mention_seen(&rec.agent_pubkey, &eid, now_secs()).ok());

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
        a["slug"].as_str().unwrap_or("").cmp(b["slug"].as_str().unwrap_or(""))
    });

    Ok(serde_json::json!({ "projects": projects }))
}

// ── project_edit ─────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9002 (edit-metadata) event signed by the human user's
/// nsec. The relay validates admin rights and updates its kind:39000 accordingly.
async fn rpc_project_edit(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
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
    let event_id = state
        .transport
        .publish_signed(builder, &user_keys)
        .await?;

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

// ── statusline ───────────────────────────────────────────────────────────────

/// How long a drained mention keeps showing on the statusline as "recently
/// consumed" before disappearing.
const STATUSLINE_RECENT_SECS: u64 = 30;

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
        let (working, _) = s.get_turn_state(&rec.session_id).unwrap_or((false, 0));
        let status = s
            .get_agent_status(&rec.agent_pubkey, &rec.project, Some(&rec.session_id))
            .ok()
            .flatten()
            .unwrap_or_default();
        let pending = s.peek_inbox(&rec.session_id).unwrap_or_default();
        let recent = s
            .list_recently_delivered(
                &rec.session_id,
                now.saturating_sub(STATUSLINE_RECENT_SECS),
            )
            .unwrap_or_default();
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
            "pending": rows_to_json(&pending, &host),
            "recent": rows_to_json(&recent, &host),
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
    let is_local = state.hosted_pubkeys().iter().any(|h| h == &original.from_pubkey);
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
            state.mention_notify.notify_waiters();
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
        let out = Command::new("git").arg("-C").arg(&dir).args(args).output().ok()?;
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
fn rpc_messages(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
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

// ── wait_for_mention (long-poll) ─────────────────────────────────────────────

async fn handle_wait_for_mention(state: &Arc<DaemonState>, req: &Request) -> Response {
    #[derive(serde::Deserialize, Default)]
    struct P {
        #[serde(default)]
        session: Option<String>,
        #[serde(default)]
        env_session: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default = "default_timeout")]
        timeout: u64,
        #[serde(default)]
        agent: Option<String>,
    }
    fn default_timeout() -> u64 {
        300
    }
    let p: P = serde_json::from_value(req.params.clone()).unwrap_or_default();
    let rec = match resolve_session(state, p.session.as_deref(), p.env_session.as_deref(), p.cwd.as_deref(), p.agent.as_deref()) {
        Ok(r) => r,
        Err(e) => return Response::err(req.id, "rpc_error", format!("{e:#}")),
    };

    // Arm the waiter so the tmux doorbell dispatcher skips this session while it
    // is actively blocked here — the agent is already listening, so there is no
    // need to type a nudge into its pane. The guard disarms on every return path.
    crate::tmux::arm_waiter(&rec.session_id);
    struct WaiterGuard(String);
    impl Drop for WaiterGuard {
        fn drop(&mut self) {
            crate::tmux::disarm_waiter(&self.0);
        }
    }
    let _waiter_guard = WaiterGuard(rec.session_id.clone());

    let _ = fetch_mentions_into_inbox(state, &rec).await;

    let deadline = if p.timeout > 0 {
        Some(tokio::time::Instant::now() + Duration::from_secs(p.timeout))
    } else {
        None
    };

    loop {
        let rows = state.with_store(|s| {
            let rows = s.drain_inbox(&rec.session_id).unwrap_or_default();
            for r in &rows {
                s.mark_mention_seen(&rec.agent_pubkey, &r.mention_event_id, now_secs())
                    .ok();
            }
            rows
        });
        if !rows.is_empty() {
            let rows_json = rows_to_json(&rows, &state.host);
            return Response::ok(req.id, serde_json::json!({ "rows": rows_json }));
        }
        // Park until a mention is routed or a short timeout for re-check.
        let wait = state.mention_notify.notified();
        let timed_out = match deadline {
            Some(d) => {
                let now = tokio::time::Instant::now();
                if now >= d {
                    true
                } else {
                    tokio::select! {
                        _ = wait => false,
                        _ = tokio::time::sleep_until(d.min(now + Duration::from_millis(500))) => {
                            tokio::time::Instant::now() >= d
                        }
                    }
                }
            }
            None => {
                wait.await;
                false
            }
        };
        if timed_out {
            return Response::ok(req.id, serde_json::json!({ "rows": [] }));
        }
    }
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
            if write_json(writer, &Response::item(id, serde_json::to_value(&ev)?)).await.is_err() {
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
    let Some(pr) = project else { return true; };
    let ev_project = match ev {
        TailEvent::Msg { project, .. } => project.as_str(),
        TailEvent::Sync { project, .. } => project.as_str(),
        TailEvent::Turn { project, .. } => project.as_str(),
        TailEvent::Status { project, .. } => project.as_str(),
        TailEvent::Join { project, .. } => project.as_str(),
        TailEvent::Leave { project, .. } => project.as_str(),
        TailEvent::Sess { project, .. } => project.as_str(),
        TailEvent::Proj { project, .. } => project.as_str(),
        // Acl and Profile are cross-project; always include.
        TailEvent::Acl { .. } | TailEvent::Profile { .. } => return true,
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
            s.recent_messages_for_backfill(project, since, limit).unwrap_or_default()
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

    // Peer sessions as synthetic Join events.
    let peers = state
        .with_store(|s| s.list_peer_sessions(project, since_peer).unwrap_or_default());
    for p in peers {
        events.push(TailEvent::Join {
            ts: p.last_seen,
            project: p.project.clone(),
            agent: p.slug.clone(),
            host: p.host.clone(),
            session: p.session_id.clone(),
            rel_cwd: p.rel_cwd.clone(),
        });
        // Add current status if known (session-scoped first, agent-level fallback).
        if let Some(text) = state.with_store(|s| {
            s.get_agent_status(&p.pubkey, &p.project, Some(&p.session_id))
                .unwrap_or(None)
        })
        {
            events.push(TailEvent::Status {
                ts: p.last_seen,
                project: p.project,
                agent: p.slug,
                text,
            });
        }
    }

    // Own sessions as synthetic Sess events.
    let mine = state
        .with_store(|s| s.list_alive_sessions().unwrap_or_default());
    for s in mine {
        if project.is_none() || project == Some(s.project.as_str()) {
            events.push(TailEvent::Sess {
                ts: s.created_at,
                project: s.project.clone(),
                agent: s.agent_slug.clone(),
                session: s.session_id.clone(),
                state: "start".into(),
                rel_cwd: s.rel_cwd.clone(),
            });
            // Add working/idle state from turn_state.
            let (working, turn_started_at) = state
                .with_store(|st| st.get_turn_state(&s.session_id).unwrap_or((false, 0)));
            if working {
                events.push(TailEvent::Turn {
                    ts: turn_started_at,
                    project: s.project.clone(),
                    agent: s.agent_slug.clone(),
                    session: s.session_id.clone(),
                    state: "working".into(),
                    elapsed_s: None,
                });
            }
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
        state.mention_notify.notify_waiters();
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
        DomainEvent::Presence(p) => {
            // Skip own sessions — local join/leave tracked by Sess events.
            if hosted.contains(&p.agent.pubkey) {
                return;
            }
            let session_id = p.session_id.as_str().to_owned();
            let is_new = {
                let mut map = state.peer_sessions.lock().unwrap();
                if !map.contains_key(&session_id) {
                    map.insert(session_id.clone(), PeerTracked {
                        first_seen: now,
                        project: p.project.clone(),
                        slug: p.agent.slug.clone(),
                        host: p.host.clone(),
                    });
                    true
                } else {
                    false
                }
            };
            if is_new {
                state.emit_tail(TailEvent::Join {
                    ts: now,
                    project: p.project.clone(),
                    agent: p.agent.slug.clone(),
                    host: p.host.clone(),
                    session: session_id,
                    rel_cwd: p.rel_cwd.clone(),
                });
            }
        }

        DomainEvent::Status(s) => {
            // Skip own status — local turn/status is tracked by Turn RPC events.
            if hosted.contains(&s.agent.pubkey) {
                return;
            }
            let key = (s.agent.pubkey.clone(), s.project.clone());
            let should_emit = {
                let mut map = state.last_status.lock().unwrap();
                let prev = map.get(&key).cloned();
                if prev.as_deref() != Some(&s.text) {
                    map.insert(key, s.text.clone());
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
                    text: s.text.clone(),
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

        DomainEvent::Activity(_) => {
            // Activity events are not emitted on the tail (they're durable
            // narrative, not real-time transitions).
        }
    }
}

// ── startup fetch of stored mentions (offline delivery) ──────────────────────

async fn fetch_mentions_into_inbox(state: &Arc<DaemonState>, rec: &crate::state::SessionRecord) -> Result<()> {
    let owners = state.owners.clone();
    let wake_count = state.provider.catch_up_mentions(rec, &owners).await?;
    if wake_count > 0 {
        state.mention_notify.notify_waiters();
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
    state
        .sessions
        .lock()
        .unwrap()
        .insert(session_id.clone(), SessionHandle { cancel: cancel.clone() });
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
    state.hosted.lock().unwrap().retain(|pk, _| live.contains(pk));
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
    let mut authors: Vec<String> = crate::acl::allowed().into_iter().collect();
    authors.extend(state.hosted_pubkeys());
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

/// Revive sessions a previous daemon left alive (skew re-exec / crash). For each
/// `alive=1` row: respawn the engine task if its `watch_pid` is still alive,
/// else mark it dead (so `who`/presence don't lie after a restart).
async fn reconcile_sessions(state: &Arc<DaemonState>) {
    let alive = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
    for rec in alive {
        let pid_ok = rec.watch_pid.map(pid_alive).unwrap_or(false);
        if !pid_ok {
            state.with_store(|s| {
                s.mark_session_dead(&rec.session_id).ok();
            });
            continue;
        }
        let id = match identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs()) {
            Ok(i) => i,
            Err(_) => continue,
        };
        // Re-establish ownership/membership + the group-state subscription for
        // revived sessions. Idempotent: the owned_groups/group_members cache
        // persists across restarts, so already-owned groups skip republishing.
        state.provider.open_project(&rec.project, &id.pubkey_hex()).await;
        if let Err(e) = ensure_subscription(state, &rec.project).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] ensure_subscription({}) failed: {e:#}", rec.project);
            }
        }
        let ep = engine_params_for(&state.cfg, &id, &rec.agent_slug, &rec.session_id, &rec.project, &rec.rel_cwd, rec.watch_pid);
        let _ = spawn_session(state, ep).await;
    }
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
        turn_repeat: Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 300)),
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
