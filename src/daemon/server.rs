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
use crate::config::{self, Config};
use crate::domain::{DomainEvent, Mention};
use crate::fabric::provider::Kind1Nip29Provider;
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, route_mention_into_with_id, EngineParams};
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, session_short_code};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

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
    provider: Kind1Nip29Provider,
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
    let provider = Kind1Nip29Provider::new(
        transport.clone(),
        store.clone(),
        cfg.user_nsec.clone(),
        &cfg.relays,
    );
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
        tail_tx: tokio::sync::broadcast::channel(256).0,
        open_clients: Mutex::new(0),
        liveness_changed: Notify::new(),
        shutdown: Notify::new(),
    });

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
        "inbox" => rpc_inbox(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params),
        "acl" => rpc_acl(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "user_prompt" => rpc_user_prompt(state, &req.params).await,
        "project_list" => rpc_project_list(state).await,
        "project_edit" => rpc_project_edit(state, &req.params).await,
        "list_threads" => rpc_list_threads(state, &req.params).await,
        "messages" => rpc_messages(state, &req.params),
        "thread_meta" => rpc_thread_meta(state, &req.params),
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
    });

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

    Ok(serde_json::json!({ "session_id": session_id }))
}


#[derive(serde::Deserialize)]
struct SessionEndParams {
    session: String,
}

fn rpc_session_end(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let existed = state.with_store(|s| s.get_session(&p.session).ok().flatten().is_some());
    if existed {
        cancel_session(state, &p.session);
        state.with_store(|s| {
            s.mark_session_dead(&p.session).ok();
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
    let intent = SendIntent {
        from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
        to_pubkey: recipient.pubkey.clone(),
        project: recipient.project.clone(),
        body: p.message,
        target_session: recipient.target_session.clone(),
        from_session: Some(rec.session_id.clone()),
        thread_id: p.thread_id.clone(),
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
    if state.hosted_pubkeys().iter().any(|h| h == &recipient.pubkey) {
        // Reconstruct the Mention for the legacy local-delivery path. Fields
        // must be byte-identical to what provider.send encoded and published.
        let mention = Mention {
            from: crate::domain::AgentRef::new(id.pubkey_hex(), rec.agent_slug.clone()),
            to_pubkey: recipient.pubkey.clone(),
            project: recipient.project.clone(),
            body,
            target_session: recipient.target_session.clone(),
            from_session: Some(rec.session_id.clone()),
        };
        let routed = state.with_store(|s| {
            route_mention_into_with_id(
                s,
                &recipient.pubkey,
                &mention,
                &receipt.native_event_id,
            )
        });
        if routed {
            state.mention_notify.notify_waiters();
        }
    }

    Ok(serde_json::json!({ "to_pubkey": recipient.pubkey, "target_session": recipient.target_session }))
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
    let rows_json = state.with_store(|s| rows_to_json(s, &rows));

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
        }
        prev
    });

    let rec = match state.with_store(|s| s.get_session(&p.session).ok().flatten()) {
        Some(r) => r,
        None => return Ok(serde_json::json!({ "context": serde_json::Value::Null })),
    };
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
    let context = crate::cli::assemble_turn_check_context(&state.store, &rec.session_id)
        .map(serde_json::Value::String)
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::json!({ "context": context }))
}

#[derive(serde::Deserialize)]
struct TurnEndParams {
    session: String,
}

fn rpc_turn_end(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: TurnEndParams =
        serde_json::from_value(params.clone()).context("parsing turn_end params")?;
    if !p.session.is_empty() {
        state.with_store(|s| {
            s.mark_turn_end(&p.session).ok();
        });
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
            crate::acl::allow(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
            });
            // Newly-trusted author: refresh the union subscription.
            resubscribe(state).await.ok();
            Ok(serde_json::json!({ "slug": slug, "pubkey": pk }))
        }
        Some("block") => {
            let target = p.target.context("acl block needs a target")?;
            let (pk, slug) = state.with_store(|s| resolve_acl_target(s, &target))?;
            crate::acl::block(&pk, &slug)?;
            state.with_store(|s| {
                s.remove_pending_agent(&pk).ok();
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
    use nostr_sdk::prelude::{Alphabet, EventBuilder, Filter, Kind, SingleLetterTag, Tag};
    let relays = state.cfg.relays.clone();
    let probe = state
        .keys_for(&state.hosted_pubkeys().first().cloned().unwrap_or_default())
        .map(|k| k.public_key().to_hex());
    let t = format!("te-doctor-{}", now_secs());
    let builder = EventBuilder::new(Kind::from(1u16), format!("tenex-edge doctor {t}"))
        .tags([Tag::parse(["h", &t])?]);
    // Sign with the daemon's connection key (any key works for the probe).
    let publish = match state.transport.publish_builder(builder).await {
        Ok(id) => format!("OK ({})", crate::util::short_id(&id.to_hex())),
        Err(e) => format!("ERR {e:#}"),
    };
    tokio::time::sleep(Duration::from_secs(1)).await;
    let f = Filter::new()
        .kind(Kind::from(1u16))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::H), &t)
        .limit(5);
    let readback = match state.transport.fetch(f, Duration::from_secs(5)).await {
        Ok(evs) => format!("{} event(s) with #h={t}", evs.len()),
        Err(e) => format!("ERR {e:#}"),
    };
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
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

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

    let builder = EventBuilder::new(Kind::from(1u16), body)
        .tags([
            Tag::parse(["h", &rec.project])?,
            Tag::parse(["p", &rec.agent_pubkey])?,
        ]);
    let event_id = state.transport.publish_signed(builder, &user_keys).await?;

    Ok(serde_json::json!({ "event_id": event_id.to_hex() }))
}

// ── project_list ─────────────────────────────────────────────────────────────

/// List NIP-29 groups: fetch all kind:39000 events from the relay (the relay
/// authors these events, so no author filter is used) and return the slug +
/// description for each. Results are also cached locally.
async fn rpc_project_list(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{Filter, Kind};

    // Phase 2: prefer local read-model rows when present (avoids a relay round-
    // trip on every call). Always attempt a relay refresh so the cache stays warm;
    // the relay result is the materialization source, not the return value.
    //
    // TODO(phase 8): when the canonical `projects` table is the write target,
    // drop the relay-fetch entirely and read only from list_projects_read_model.
    let local = state
        .with_store(|s| s.list_projects_read_model())
        .unwrap_or_default();

    // Kick off relay fetch for materialization regardless of local cache state.
    let filter = Filter::new().kind(Kind::from(39000u16)).limit(200);
    let events = state
        .transport
        .fetch(filter, Duration::from_secs(5))
        .await
        .unwrap_or_default();

    let now = now_secs();
    let mut from_relay: Vec<serde_json::Value> = Vec::new();
    for ev in &events {
        let Some(slug) = event_tag(ev, "d") else {
            continue;
        };
        let about = event_tag(ev, "about").unwrap_or("").to_string();
        state.with_store(|s| {
            // Materialization refresh: update legacy project_meta cache.
            s.upsert_project_meta(slug, &about, now).ok();
        });
        from_relay.push(serde_json::json!({ "slug": slug, "about": about }));
    }

    // Prefer relay-derived list (freshest); fall back to local cache when the
    // relay returns nothing (offline / timeout / empty relay).
    let mut projects = if !from_relay.is_empty() {
        from_relay
    } else {
        local
            .into_iter()
            .map(|(slug, about)| serde_json::json!({ "slug": slug, "about": about }))
            .collect()
    };
    projects.sort_by(|a, b| {
        a["slug"].as_str().unwrap_or("").cmp(b["slug"].as_str().unwrap_or(""))
    });

    Ok(serde_json::json!({ "projects": projects }))
}

// ── project_edit ─────────────────────────────────────────────────────────────

/// Publish a NIP-29 kind:9002 (edit-metadata) event signed by the human user's
/// nsec. The relay validates admin rights and updates its kind:39000 accordingly.
async fn rpc_project_edit(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    use nostr_sdk::prelude::{EventBuilder, Keys, Kind, Tag};

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

    // kind:9002 = NIP-29 edit-metadata. The relay validates admin rights and
    // re-publishes kind:39000 signed by the relay key.
    let builder = EventBuilder::new(Kind::from(9002u16), "")
        .tags([
            Tag::parse(["d", &p.project])?,
            Tag::parse(["about", &p.description])?,
        ]);
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
    Ok(serde_json::to_value(&meta)?)
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
            let rows_json = state.with_store(|s| rows_to_json(s, &rows));
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

async fn handle_tail<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let project = params.get("project").and_then(|v| v.as_str()).map(str::to_string);
    // Ensure the requested project is in the union subscription so its events
    // flow through the shared connection.
    if let Some(pr) = &project {
        let _ = ensure_subscription(state, pr).await;
    }
    let mut rx = state.tail_subscribe();
    {
        *state.open_clients.lock().unwrap() += 1;
        state.liveness_changed.notify_waiters();
    }
    let _guard = ClientGuard(state.clone());

    loop {
        match rx.recv().await {
            Ok(de) => {
                if let Some(line) = render_fabric_line(&de, project.as_deref()) {
                    if write_json(writer, &Response::item(id, serde_json::json!({ "line": line }))).await.is_err() {
                        break; // client disconnected
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

/// Render a fabric event for `tail`, scoped to `project` if given. Mirrors the
/// old CLI `render()` output so `tail` looks identical.
fn render_fabric_line(de: &DomainEvent, project: Option<&str>) -> Option<String> {
    if let Some(pr) = project {
        let matches = match de {
            DomainEvent::Presence(p) => p.project == pr,
            DomainEvent::Activity(a) => a.project == pr,
            DomainEvent::Status(s) => s.project == pr,
            DomainEvent::Mention(m) => m.project == pr,
            DomainEvent::Profile(_) => true,
        };
        if !matches {
            return None;
        }
    }
    Some(crate::cli::render_fabric(de))
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
/// Thin dispatch to `provider.materialize` (Phase 5).
fn handle_incoming(state: &Arc<DaemonState>, event: &Event) {
    let env = crate::fabric::RawEnvelope::Nostr(event.clone());
    let hosted = state.hosted_pubkeys();
    let owners = state.owners.clone();
    let now = now_secs();
    let outcome = state.with_store(|s| state.provider.materialize(&env, &hosted, &owners, now, s));
    if let Some(de) = outcome.tail {
        let _ = state.tail_tx_send(de);
    }
    if outcome.wake_mentions {
        state.mention_notify.notify_waiters();
    }
}

fn event_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event.tags.iter().find_map(|t| {
        let s = t.as_slice();
        if s.first().map(String::as_str) == Some(name) {
            s.get(1).map(String::as_str)
        } else {
            None
        }
    })
}

// ── startup fetch of stored mentions (offline delivery) ──────────────────────

async fn fetch_mentions_into_inbox(state: &Arc<DaemonState>, rec: &crate::state::SessionRecord) -> Result<()> {
    let owners = state.owners.clone();
    let wake_count = state.provider.catch_up_mentions(rec, &owners).await?;
    if wake_count > 0 {
        state.mention_notify.notify_waiters();
    }
    Ok(())
}

// ── pruner ───────────────────────────────────────────────────────────────────

fn spawn_pruner(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(30));
        loop {
            tick.tick().await;
            let before = now_secs().saturating_sub(PRUNE_PEER_AFTER_SECS);
            state.with_store(|s| {
                let _ = s.prune_peer_sessions(before);
            });
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
    let transport = state.transport.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(params, transport, store, cancel).await;
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

impl DaemonState {
    fn tail_subscribe(&self) -> tokio::sync::broadcast::Receiver<DomainEvent> {
        self.tail_tx.subscribe()
    }
    fn tail_tx_send(&self, de: DomainEvent) -> Result<usize, tokio::sync::broadcast::error::SendError<DomainEvent>> {
        self.tail_tx.send(de)
    }
}

fn rows_to_json(store: &Store, rows: &[InboxRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|r| {
            serde_json::json!({
                "from_slug": r.from_slug,
                "project": r.project,
                "body": r.body,
                "mention_event_id": r.mention_event_id,
                "from_session": r.from_session,
                // Fully-qualified handle the receiver passes to `--recipient`.
                "reply_to": crate::cli::mention_reply_handle(store, r),
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
