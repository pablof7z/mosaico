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
use crate::domain::{ChatMessage, DomainEvent};
use crate::fabric::provider::Nip29Provider;
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, EngineParams};
use crate::session::{derive_status, Harness, SessionObservation, SessionSnapshot};
use crate::state::{ChatInboxRow, ChatLogRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, pubkey_short, session_codename};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

mod demux;
mod rpc;
mod session_signer;
mod tmux_rpc;

use demux::{handle_orchestration, spawn_demux};

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
    provider: Arc<Nip29Provider>,
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
    /// Peer presence join/leave tracking, keyed by `(pubkey, project)` — peers
    /// no longer carry a session id, and a durable agent can be present in
    /// several projects at once, so identity is `(pubkey, group)`.
    peer_sessions: Mutex<HashMap<(String, String), PeerTracked>>,
    /// Bounded first-sight tracking of native event ids: the relay pool
    /// notifies once per matching subscription, so the same event arrives many
    /// times. Set + insertion-order queue, capped at SEEN_EVENTS_CAP.
    seen_events: Mutex<(
        std::collections::HashSet<String>,
        std::collections::VecDeque<String>,
    )>,
    /// Pubkeys for which a Profile event has already been emitted, for first-seen dedup.
    seen_profiles: Mutex<std::collections::HashSet<String>>,
    /// Last-seen (title, active) keyed by `(author_pubkey, group_id)` for tail
    /// dedup. All sessions of a durable agent in one project sign with the same
    /// key and address the same replaceable slot, so per-agent/group dedup is
    /// correct. Tracking `active` too means an active→idle flip emits a tail
    /// event even though the persistent title text is unchanged.
    last_status: Mutex<HashMap<(String, String), (String, bool)>>,
    /// Wakes the status-outbox drainer the instant a transition enqueues a publish.
    status_outbox_notify: Notify,
    /// Per-session derived keypairs for duplicate live signers. The durable
    /// agent key remains the default; this map is populated only when a second
    /// live session of the same durable agent joins the same routing scope.
    session_keys: Mutex<HashMap<String, Keys>>,
    /// Reserved durable signer slots keyed by `(durable agent pubkey, group)`.
    /// Guards collision detection and reservation so simultaneous duplicate
    /// starts cannot both pick the durable signer.
    session_signers: Mutex<session_signer::SignerReservations>,
    /// Hex pubkey of this backend's identity (pubkey of `tenexPrivateKey`;
    /// no `userNsec` fallback). Added as an admin to every group we create
    /// and the address the subgroup orchestration listener matches `add` tags
    /// against. `None` only when no `tenexPrivateKey` is configured.
    backend_pubkey: Option<String>,
}

impl DaemonState {
    /// Hex pubkey of this backend's identity key, if configured.
    fn backend_pubkey(&self) -> Option<&str> {
        self.backend_pubkey.as_deref()
    }
    pub(crate) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
    /// The shared relay connection. Used by the kind:0 profile resolver to
    /// one-shot fetch a pubkey's metadata on a cache miss.
    pub(crate) fn transport(&self) -> &Arc<Transport> {
        &self.transport
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
    /// Retrieve the derived per-session keypair by canonical session id.
    /// Callers fall back to the durable agent key via `keys_for` when absent.
    fn keys_for_session(&self, session_id: &str) -> Option<Keys> {
        self.session_keys.lock().unwrap().get(session_id).cloned()
    }
    fn live_session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
    /// Return live per-session derived pubkeys. Callers in `resubscribe` and
    /// `handle_incoming` extend their sets with this so transient duplicates are
    /// subscribed and recognized as local authors/recipients.
    fn live_session_pubkeys(&self) -> Vec<String> {
        self.session_keys
            .lock()
            .unwrap()
            .values()
            .map(|k| k.public_key().to_hex())
            .collect()
    }
    fn release_session_signer(
        &self,
        session_id: &str,
        agent_pubkey: &str,
        project: &str,
    ) -> Option<Keys> {
        let mut reservations = self.session_signers.lock().unwrap();
        let mut session_keys = self.session_keys.lock().unwrap();
        session_signer::release(
            &mut reservations,
            &mut session_keys,
            session_id,
            agent_pubkey,
            project,
        )
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
    let provider = Arc::new(Nip29Provider::new(
        transport.clone(),
        store.clone(),
        cfg.management_nsec().cloned(),
        cfg.user_nsec().cloned(),
        cfg.whitelisted_pubkeys.clone(),
        &cfg.relays, // provider_instance hashes main relays only, not indexer
    ));
    // Backend identity: pubkey of `tenexPrivateKey` (no `userNsec` fallback —
    // the operator key is a human identity, not a backend identity). Used as a
    // copied admin on every group we create and as the orchestration listener's
    // `add`-tag matcher.
    let backend_pubkey: Option<String> = cfg
        .backend_nsec()
        .and_then(|n| Keys::parse(n).ok())
        .map(|k| k.public_key().to_hex());

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
        session_keys: Mutex::new(HashMap::new()),
        session_signers: Mutex::new(HashMap::new()),
        backend_pubkey,
    });

    // Idempotent read-model backfill: populate canonical `projects` + `membership`
    // tables from legacy data so readers have a consistent origin on every start.
    // Best-effort: a backfill error must not prevent startup.
    {
        let pi = state.provider.provider_instance.clone();
        state.with_store(|s| {
            s.backfill_nip29_origins(&pi, now_secs()).ok();
        });
    }

    // These tolerate a not-yet-connected relay (demux just waits for events;
    // publishers/subscribers are best-effort and queue), so they start now.
    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    spawn_idle_watcher(state.clone());
    spawn_status_outbox_drainer(state.clone());
    spawn_status_heartbeat_publisher(state.clone());
    spawn_retry_drainer(state.clone());

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

    // Relay-DEPENDENT startup runs in the background, OFF the accept path, so the
    // daemon serves store-only RPCs (`who`, `tmux`, chat/inbox reads, statusline,
    // whoami) immediately even when the relay is slow or unreachable. We warm up
    // the connection (await connectivity + NIP-42 auth) BEFORE opening any
    // subscription — a REQ opened pre-auth on an auth-gated relay never delivers.
    let relay_state = state.clone();
    tokio::spawn(async move {
        relay_state.transport.warmup().await;

        // Publish the backend's own kind:0 so it is identifiable on the relay by
        // Nostr clients. Best-effort: failure deferred to next restart.
        // Intentionally NOT stored in the hosted set — the echo must NOT appear in
        // `who` or be injected into agent turn-context.
        if let Some(nsec) = relay_state.cfg.backend_nsec() {
            if let Ok(backend_keys) = nostr_sdk::prelude::Keys::parse(nsec) {
                let name = format!("{} (tenex-edge)", relay_state.host);
                let ev = crate::domain::DomainEvent::Profile(crate::domain::Profile {
                    agent: crate::domain::AgentRef::new(backend_keys.public_key().to_hex(), name),
                    host: relay_state.host.clone(),
                    owners: relay_state.owners.clone(),
                    is_backend: true,
                });
                let _ = relay_state.provider.publish(&ev, &backend_keys).await;
            }
        }

        // Standalone backend orchestration subscription (kind:9 p-tagged to this
        // backend's identity), independent of any project — so a backend with no
        // live sessions still receives subgroup add-agents requests (issue #3).
        if let Some(bp) = relay_state.backend_pubkey() {
            if let Err(e) = relay_state
                .provider
                .subscribe_backend_orchestration(bp)
                .await
            {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!("[daemon] backend orchestration subscription failed: {e:#}");
                }
            }
        }

        // Subscribe to groups where local agents are already members so kind:9
        // chat arrives even when no session is alive (spawn-on-mention path).
        // Each group gets its own #h-filtered subscription via ensure_subscription;
        // new memberships discovered from 39002 events extend coverage dynamically.
        {
            let edge = crate::config::edge_home();
            let local_pks: Vec<String> = crate::identity::list_local_pubkeys(&edge);
            let member_groups: Vec<String> = relay_state.with_store(|s| {
                let mut groups = Vec::new();
                for pk in &local_pks {
                    if let Ok(gs) = s.list_groups_for_member(pk) {
                        groups.extend(gs);
                    }
                }
                groups.sort_unstable();
                groups.dedup();
                groups
            });
            for group in &member_groups {
                let _ = ensure_subscription(&relay_state, group).await;
            }
            eprintln!(
                "[daemon] spawn-on-mention: {} local agents, {} member groups subscribed",
                local_pks.len(),
                member_groups.len()
            );
        }

        // Revive sessions a previous daemon left behind + (re)open their project
        // subscriptions. Subscriptions go out post-auth.
        reconcile_sessions(&relay_state).await;
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
        "chat_write" => rpc_chat_write(state, &req.params).await,
        "user_prompt" => rpc_user_prompt(state, &req.params).await,
        "publish" => rpc_propose(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "local_backend" => rpc_local_backend(state),
        "project_list" => rpc::rpc_project_list(state).await,
        "project_edit" => rpc::rpc_project_edit(state, &req.params).await,
        "project_members" => rpc::rpc_project_members(state, &req.params).await,
        "project_add" => rpc::rpc_project_add(state, &req.params).await,
        "project_remove" => rpc::rpc_project_remove(state, &req.params).await,
        "debug_outbox" => rpc_debug_outbox(state, &req.params),
        "channels_create" => rpc_channels_create(state, &req.params).await,
        "channels_list" => rpc_channels_list(state, &req.params),
        "channels_switch" => rpc_channels_switch(state, &req.params).await,
        "publish_profile" => rpc_publish_profile(state, &req.params).await,
        "statusline" => rpc_statusline(state, &req.params),
        "whoami" => rpc_whoami(state, &req.params),
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
    group: Option<&str>,
) -> Result<crate::state::SessionRecord> {
    resolve_session_inner(state, explicit, env_session, cwd, agent, group, true)
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
    group: Option<&str>,
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
    // A subgroup session is stored under its child group id (`h`), not the
    // working-directory project. When the caller is inside such a session its
    // pane exports `TENEX_EDGE_CHANNEL`; prefer it over the cwd-derived project so
    // the (agent, project) lookup finds the subgroup session rather than a
    // sibling parent-project session.
    // When the caller carries an explicit group, that group id IS the project a
    // session is stored under (a `tenex-edge launch`-spawned subgroup session).
    // Otherwise the project is the bare work-root from `cwd` — and a
    // human-initiated session is stored under a per-session room minted beneath
    // it (issue #6), so an exact `project` match misses. `work_root` drives the
    // room-aware fallback below for exactly that case.
    let explicit_group = group.filter(|g| !g.is_empty());
    let work_root = explicit_group.is_none();
    let project = explicit_group
        .map(|g| g.to_string())
        .unwrap_or_else(|| crate::project::resolve(&cwd).unwrap_or_default());
    if let Some(agent) = agent.filter(|a| !a.is_empty()) {
        if let Some(rec) =
            state.with_store(|s| s.latest_alive_session_for_agent_in_project(agent, &project))?
        {
            return Ok(rec);
        }
        if work_root {
            if let Some(rec) = state
                .with_store(|s| s.latest_alive_session_under_work_root(&project, Some(agent)))?
            {
                return Ok(rec);
            }
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
    if let Some(rec) = state.with_store(|s| s.latest_alive_session_for_project(&project))? {
        return Ok(rec);
    }
    if work_root {
        if let Some(rec) =
            state.with_store(|s| s.latest_alive_session_under_work_root(&project, None))?
        {
            return Ok(rec);
        }
    }
    anyhow::bail!(
        "no active tenex-edge session for project {project:?} (run session-start, or pass --session)"
    )
}

// ── who ──────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct WhoParams {
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    all_projects: bool,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
}

/// `who`: build the snapshot with the SAME function the CLI used. The client
/// renders it with the existing renderers, so output is byte-identical. The
/// daemon resolves the current project the same way the old CLI did.
fn rpc_who(state: &Arc<DaemonState>, params: &serde_json::Value) -> Result<serde_json::Value> {
    let p: WhoParams = serde_json::from_value(params.clone()).unwrap_or_default();
    let current_project = if p.all_projects {
        None
    } else if p.project.is_none()
        && (p.env_session.as_deref().filter(|s| !s.is_empty()).is_some()
            || p.agent.as_deref().filter(|s| !s.is_empty()).is_some()
            || p.group.as_deref().filter(|s| !s.is_empty()).is_some())
    {
        Some(
            resolve_session_inner(
                state,
                None,
                p.env_session.as_deref(),
                p.cwd.as_deref(),
                p.agent.as_deref(),
                p.group.as_deref(),
                false,
            )
            .map(|rec| rec.route_scope().to_string())?,
        )
    } else {
        Some(p.project.clone().unwrap_or_else(|| {
            let cwd = p
                .cwd
                .clone()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            crate::project::resolve(&cwd).unwrap_or_default()
        }))
    };
    let now = now_secs();
    let host = state.host.clone();
    let snapshot = state
        .with_store(|s| crate::cli::load_who_snapshot(s, current_project.as_deref(), now, &host))?;
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
    /// NIP-29 subgroup id (`h`) this pane was spawned into (from
    /// `TENEX_EDGE_CHANNEL`). When present, the session is scoped to this channel
    /// instead of the working-directory project: all channel publishing
    /// (presence/status/chat/mentions/membership) keys on it. The working
    /// directory remains the parent repo. Absent for ordinary project sessions.
    #[serde(default)]
    channel: Option<String>,
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
    // The working-directory project (the repo this harness runs in).
    let work_root = crate::project::resolve(&cwd).unwrap_or_default();
    // The NIP-29 channel this session belongs to. For a subgroup task room this is
    // the child `h` supplied via TENEX_EDGE_CHANNEL; otherwise it equals the
    // working-directory project (continuity: existing sessions are unchanged).
    // Everything below keys group membership + fabric publishing on `project`.
    let mut project = p
        .channel
        .clone()
        .filter(|g| !g.is_empty())
        .unwrap_or_else(|| work_root.clone());
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

    // Per-session rooms (issue #6): a human-initiated session — one with no
    // TENEX_EDGE_CHANNEL override (someone ran `claude` / `tenex-edge launch`
    // directly) — lives in its OWN minted subgroup of the work-root project,
    // not the bare project. Orchestration-spawned sessions (group override
    // present) join the supplied subgroup as today. The room id is derived
    // deterministically from a stable per-session anchor so a resumed session
    // rejoins the SAME room; minting needs an operator key to sign the create.
    //
    // Anchor preference: the harness-native id (claude/codex) or resume token,
    // else the watched pid — opencode supplies neither id nor resume token at
    // start (only its pid), so the pid keeps it from being left in the bare
    // project. `room_parent` is `Some(parent_project)` exactly when we routed
    // the session into a freshly-minted room, and drives the create below.
    let pid_anchor = p.watch_pid.map(|pid| format!("pid-{pid}"));
    let room_parent: Option<String> = {
        let anchor = harness_session_id
            .as_deref()
            .or(resume_id.as_deref())
            .or(pid_anchor.as_deref());
        match (
            crate::session::decide_session_room(p.channel.as_deref(), &work_root),
            anchor,
        ) {
            (crate::session::RoomDecision::Mint { parent }, Some(anchor)) => {
                project = crate::util::session_room_id(anchor);
                Some(parent)
            }
            _ => None,
        }
    };

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

    // The session's first kind:30315 row was enqueued by
    // register_or_reassert_session above. Do not wake the drainer until signer
    // selection below has reserved durable vs transient identity for this scope.

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
            if rec.session_id == session_id || rec.agent_slug != p.agent || rec.host != state.host {
                continue;
            }
            let same_work_root = state
                .with_store(|s| {
                    let old_root = s.work_root_for_scope(&rec.project)?;
                    let new_root = room_parent.clone().unwrap_or_else(|| {
                        s.work_root_for_scope(&project).unwrap_or(project.clone())
                    });
                    Ok::<bool, anyhow::Error>(old_root == new_root)
                })
                .unwrap_or(rec.project == project);
            if !same_work_root {
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
            channel: p.channel.clone().unwrap_or_default(),
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

    // Stamp the canonical session id onto the tmux session owning this pane so
    // the status-format `#(...)` can read it via `#{@te_session}` and pass
    // `--session` to `tenex-edge statusline`. Without this, two panes of the
    // same agent in the same project collapse to a single status bar (the
    // `#(...)` runs in the tmux server's env, which can't see the pane's
    // TENEX_EDGE_SESSION). Best-effort and deliberately outside the store lock.
    //
    // When the re-registration arrives without TMUX_PANE (e.g. a reassert from
    // a non-tmux context after a daemon restart), fall back to the session's
    // existing tmux endpoint so @te_session is never left stale.
    let effective_pane = tmux_pane.clone().or_else(|| {
        state
            .with_store(|s| s.get_session_endpoint(&session_id, "tmux"))
            .ok()
            .flatten()
            .map(|ep| ep.target)
    });
    if let Some(ref pane) = effective_pane {
        crate::tmux::set_pane_session_id(pane, &session_id, p.tmux_socket.as_deref());
    }

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
    if let Some(parent) = &room_parent {
        // Human-initiated session: mint its per-session room under the work-root.
        // Mark the room in the LOCAL read-model SYNCHRONOUSLY (so the
        // room/subgroup gates and `groups list` recognize it immediately), then
        // do all the relay-dependent work (parent open, subgroup create, admin
        // reflection poll, member-add) in the BACKGROUND. This keeps session
        // start — and thus the first prompt — off the relay's critical path
        // entirely (fail-open). Chat into the room before the relay mint lands is
        // best-effort and simply not mirrored until the room exists.
        if let Some(prog) = &progress {
            prog.emit("nip29", format!("minting per-session room {project}"));
        }
        let now = now_secs();
        state.with_store(|s| {
            s.mark_session_room(&project, parent, now).ok();
            s.upsert_group_metadata(&project, &project, parent, now)
                .ok();
        });
        // ALL relay work (subgroup create, admin poll, member-add, subscription)
        // runs in the background — session start has zero synchronous relay await
        // on the room path, so a slow/unreachable relay never delays the engine
        // or the first prompt. ensure_session_room subscribes internally.
        let st = state.clone();
        let room = project.clone();
        let par = parent.clone();
        let name = project.clone();
        let agent_pubkey = id.pubkey_hex();
        tokio::spawn(async move {
            ensure_session_room(&st, &room, &name, &par, &agent_pubkey).await;
        });
    } else {
        // Project / orchestration sessions: ensure the group exists + the agent
        // is a member. Bounded so a hung relay can't block session start (and the
        // hook that awaits it). On timeout the session still starts; membership
        // converges on the next start/heartbeat. Best-effort, fail-open.
        let open = async {
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
        };
        if tokio::time::timeout(std::time::Duration::from_secs(8), open)
            .await
            .is_err()
            && std::env::var("TENEX_EDGE_DEBUG").is_ok()
        {
            eprintln!("[daemon] open_project({project}) timed out (best-effort)");
        }
    }
    let (harness_kind, anchor) = state.with_store(|s| s.get_session_derivation_anchor(&session_id));
    let signer = select_session_signer(
        state,
        &session_id,
        &id.pubkey_hex(),
        &p.agent,
        &project,
        &harness_kind,
        &anchor,
    )?;
    if let Some(session_pubkey) = signer.transient_pubkey() {
        if let Some(prog) = &progress {
            prog.emit(
                "nip29",
                format!(
                    "admitting transient signer {} before routing use",
                    pubkey_short(session_pubkey)
                ),
            );
        }
        if let Err(e) = admit_transient_signer(state, &project, session_pubkey).await {
            state.release_session_signer(&session_id, &id.pubkey_hex(), &project);
            state.with_store(|s| s.remove_session_pubkeys_for_session(&session_id).ok());
            return Err(e);
        }
    }

    // Nudge the drainer now that signer selection/admission is complete: the
    // pending first kind:30315 must be signed by the selected identity.
    state.status_outbox_notify.notify_waiters();

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
        signer.session_keys(),
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

        // Release durable-slot reservation and any transient key before marking
        // the session dead. Fire-and-forget relay removal keeps session_end fast;
        // spawn_session cleanup will find the key gone and skip duplicate work.
        let session_key =
            state.release_session_signer(&rec.session_id, &rec.agent_pubkey, &rec.project);
        if let Some(sk) = session_key {
            let provider = state.provider.clone();
            let store = state.store.clone();
            // Remove from the session's CURRENT routing scope — its channel when
            // set (a `channels switch` moved it), else its per-session room — so
            // the NIP-29 member removal lands in the group the relay actually has
            // the agent in, not the room it minted at spawn.
            let scope = rec.route_scope().to_string();
            let session_pubkey = sk.public_key().to_hex();
            tokio::spawn(async move {
                let removed = provider.nip29_remove_member(&scope, &session_pubkey).await;
                // Mirror into the cache unconditionally: relay rejection of a
                // remove for a non-member is benign (idempotent), so always
                // clean up our local row to avoid stale membership.
                store
                    .lock()
                    .unwrap()
                    .remove_group_member(&scope, &session_pubkey)
                    .ok();
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] session-end NIP-29 remove {}: {}",
                        crate::util::pubkey_short(&session_pubkey),
                        if removed {
                            "accepted"
                        } else {
                            "skipped/failed (best-effort)"
                        },
                    );
                }
            });
        }

        state.with_store(|s| {
            // Finish the canonical aggregate (lifecycle=ended; title retained) so
            // the session surfaces as a 'gone' delta, AND mark the kept runtime row
            // dead. The final publish carries a fresh expiration and ages off.
            s.end_session(&rec.session_id, now_secs()).ok();
            s.mark_session_dead(&rec.session_id).ok();
            // Clear the DB routing row for this session's transient pubkey.
            s.remove_session_pubkeys_for_session(&rec.session_id).ok();
        });
        state.status_outbox_notify.notify_waiters();
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            project: rec.route_scope().to_string(),
            agent: rec.agent_slug.clone(),
            session: rec.session_id.clone(),
            state: "end".into(),
            rel_cwd: rec.rel_cwd.clone(),
        });
    }
    Ok(serde_json::json!({ "ended": existed }))
}

fn select_session_signer(
    state: &Arc<DaemonState>,
    session_id: &str,
    agent_pubkey: &str,
    agent_slug: &str,
    project: &str,
    harness_kind: &str,
    anchor: &str,
) -> Result<session_signer::SessionSigner> {
    let existing_session_pubkey = state.with_store(|s| s.session_pubkey_for_session(session_id));
    let tenex_keys = match state.cfg.session_ikm_nsec() {
        Some(nsec) => Some(
            Keys::parse(nsec)
                .context("parsing tenexPrivateKey for transient session signer derivation")?,
        ),
        None => None,
    };
    let signer = {
        let mut reservations = state.session_signers.lock().unwrap();
        let mut session_keys = state.session_keys.lock().unwrap();
        session_signer::select_and_reserve(
            &mut reservations,
            &mut session_keys,
            session_signer::SignerRequest {
                session_id,
                agent_pubkey,
                agent_slug,
                project,
                harness_kind,
                anchor,
                existing_session_pubkey,
                tenex_secret: tenex_keys.as_ref().map(Keys::secret_key),
            },
        )?
    };
    if let Some(session_pubkey) = signer.transient_pubkey() {
        if let Err(e) = state.with_store(|s| {
            s.upsert_session_pubkey(
                session_pubkey,
                session_id,
                agent_pubkey,
                agent_slug,
                now_secs(),
            )
        }) {
            state.release_session_signer(session_id, agent_pubkey, project);
            return Err(e);
        }
    }
    Ok(signer)
}

async fn admit_transient_signer(
    state: &Arc<DaemonState>,
    project: &str,
    session_pubkey: &str,
) -> Result<()> {
    let add = state.provider.nip29_add_member(project, session_pubkey);
    let accepted = tokio::time::timeout(std::time::Duration::from_secs(8), add)
        .await
        .unwrap_or(false);
    if !accepted {
        anyhow::bail!(
            "NIP-29 admission failed for transient signer {} in {project}",
            pubkey_short(session_pubkey)
        );
    }
    state.with_store(|s| s.upsert_group_member(project, session_pubkey, "member", now_secs()))?;
    Ok(())
}

// ── chat_write ───────────────────────────────────────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ChatWriteParams {
    message: String,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
}

#[derive(Clone)]
struct ChatRecordDraft {
    from_pubkey: String,
    from_slug: String,
    host: String,
    project: String,
    body: String,
    from_session: String,
    mentioned_session: String,
}

async fn publish_chat_checked(
    state: &Arc<DaemonState>,
    chat: &ChatMessage,
    signing: &Keys,
    draft: &ChatRecordDraft,
) -> Result<String> {
    let builder = state
        .provider
        .encode(&DomainEvent::ChatMessage(chat.clone()))?;
    let signed = state.transport.sign(builder, signing).await?;
    let event_id = signed.id.to_hex();
    let created_at = now_secs();

    state.with_store(|s| {
        let _ = s.record_chat(&ChatLogRow {
            chat_event_id: event_id.clone(),
            from_pubkey: draft.from_pubkey.clone(),
            from_slug: draft.from_slug.clone(),
            host: draft.host.clone(),
            project: draft.project.clone(),
            body: draft.body.clone(),
            created_at,
            from_session: draft.from_session.clone(),
            mentioned_session: draft.mentioned_session.clone(),
        });
    });

    state.transport.publish_event(&signed).await?;
    Ok(event_id)
}

fn spawn_chat_publish_retry(
    state: Arc<DaemonState>,
    chat: ChatMessage,
    signing: Keys,
    draft: ChatRecordDraft,
    label: &'static str,
) {
    tokio::spawn(async move {
        let mut last_err = String::new();
        for attempt in 0..60 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
            match publish_chat_checked(&state, &chat, &signing, &draft).await {
                Ok(_) => return,
                Err(e) => last_err = e.to_string(),
            }
        }
        eprintln!("[daemon] {label} kind:9 publish retry exhausted: {last_err}");
    });
}

fn spawn_retry_drainer(state: Arc<DaemonState>) {
    let queue = state.transport.retry_queue.clone();
    let transport = state.transport.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let due = queue.drain_due();
            for retry in due {
                let id_short = {
                    let h = retry.event.id.to_hex();
                    h[..8.min(h.len())].to_string()
                };
                let kind = retry.event.kind.as_u16();
                if transport.retry_publish(&retry.event).await {
                    eprintln!(
                        "[retry] event {id_short} kind:{kind} accepted on attempt {}",
                        retry.attempt + 1
                    );
                } else {
                    queue.requeue(retry, "relay rejected on retry");
                }
            }
        }
    });
}

fn nip29_retry_delay(attempt: u32) -> std::time::Duration {
    let exponent = attempt.min(4);
    let delay_ms = 500_u64.saturating_mul(2_u64.saturating_pow(exponent));
    std::time::Duration::from_millis(delay_ms.min(8_000))
}

async fn apply_room_name_update(state: &Arc<DaemonState>, room: &str, title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    let should_publish = state.with_store(|s| {
        let owned =
            s.is_session_room(room).unwrap_or(false) && s.is_group_owned(room).unwrap_or(false);
        let current = s.group_display_name(room).unwrap_or_default();
        owned && current.trim() != title
    });
    if !should_publish {
        return false;
    }
    let renamed = state.provider.nip29_set_group_name(room, title).await;
    if renamed {
        let parent = state
            .with_store(|s| s.session_room_parent(room).ok().flatten())
            .unwrap_or_default();
        state.with_store(|s| {
            s.upsert_group_metadata(room, title, &parent, now_secs())
                .ok();
        });
    }
    renamed
}

fn spawn_room_name_update(state: Arc<DaemonState>, room: String, title: String) {
    tokio::spawn(async move {
        apply_room_name_update(&state, &room, &title).await;
    });
}

/// Publish the agent's completed-turn output as kind:9 chat into the session's
/// room (issue #6). Signed by the agent via `keys_for_session`, which falls
/// back to the durable agent key (#5). Mirrors `rpc_chat_write`'s publish +
/// local record, minus mention handling.
async fn publish_agent_reply(
    state: &Arc<DaemonState>,
    rec: &crate::state::SessionRecord,
    reply: &str,
) -> Result<()> {
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    // Sign with the selected session identity: durable by default, transient
    // only when this live session collided with another durable signer in the
    // same routing scope.
    let signing = state
        .keys_for_session(&rec.session_id)
        .unwrap_or_else(|| id.keys.clone());
    let from_pubkey = signing.public_key().to_hex();
    // Route into the session's CURRENT scope (channel when set, else the
    // per-session room) so a `channels switch` moves the agent's turn replies
    // to the new room without restarting.
    let scope = rec.route_scope().to_string();

    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(from_pubkey.clone(), rec.agent_slug.clone()),
        project: scope.clone(),
        body: reply.to_string(),
        mentioned_pubkey: None,
    };
    let draft = ChatRecordDraft {
        from_pubkey,
        from_slug: rec.agent_slug.clone(),
        host: state.host.clone(),
        project: scope,
        body: reply.to_string(),
        from_session: rec.session_id.clone(),
        mentioned_session: String::new(),
    };
    let publish = publish_chat_checked(state, &chat, &signing, &draft);
    match tokio::time::timeout(std::time::Duration::from_secs(3), publish).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) | Err(_) => {
            spawn_chat_publish_retry(state.clone(), chat, signing, draft, "agent reply");
        }
    }
    Ok(())
}

/// Publish a user's prompt as kind:9 chat into the session's room (issue #6).
///
/// The human is speaking, so the event is signed by the OPERATOR key (which is
/// the room's admin) rather than the agent/session key. Fail-open: if no
/// operator key is set the prompt is simply not mirrored — the hook must never
/// block the editor. The session resolves to its room via `resolve_session`,
/// so the prompt lands in the same per-session subgroup the agent posts into.
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
        agent: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        prompt: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("parsing user_prompt params")?;
    if p.prompt.trim().is_empty() {
        return Ok(serde_json::json!({ "skipped": "empty prompt" }));
    }

    // A daemon-injected fabric envelope (a mention pasted into the pane by the
    // tmux delivery path) is ALREADY a kind:9 event in the room. The harness
    // re-submits it as a prompt, firing user-prompt-submit, but republishing it
    // would echo the message back into the channel (and, on publish timeout +
    // retry, twice). Only genuine human keyboard input is mirrored.
    if crate::injection::is_fabric_injection(&p.prompt) {
        return Ok(serde_json::json!({ "skipped": "fabric injection echo" }));
    }

    // No operator key → nothing to sign with; fail open (session still runs).
    // `userNsec` is the ONLY signer for user prompts — the human is speaking.
    let Some(nsec) = state.cfg.user_nsec() else {
        return Ok(serde_json::json!({ "skipped": "userNsec unset" }));
    };
    let op_keys = Keys::parse(nsec).context("parsing operator key")?;
    let op_pubkey = op_keys.public_key().to_hex();

    let rec = resolve_session(
        state,
        p.session.as_deref(),
        p.env_session.as_deref(),
        p.cwd.as_deref(),
        p.agent.as_deref(),
        None,
    )?;

    // Only mirror prompts into a per-session room. A human start with no resume
    // anchor (or no operator key) keeps `project == work_root`; mirroring there
    // would spam the bare project group, so skip it (fail-open). Gate on the
    // local `is_session_room` flag — set synchronously at mint and never touched
    // by the relay materializer (unlike `project_meta.parent`, which a relay that
    // doesn't re-emit the NIP-29 parent tag can clobber to empty).
    let is_room = state
        .with_store(|s| s.is_session_room(&rec.project))
        .unwrap_or(false);
    if !is_room {
        return Ok(serde_json::json!({ "skipped": "session not in a room" }));
    }

    // Seed the canonical session title once, from the real user prompt, so the
    // status pipeline has an immediate title before the runtime transcript read
    // catches up. Room metadata is a consequence of a title transition, not a
    // per-hook mirror: publish a room-name update only when this call actually
    // changed session_state.
    let seeded_title = state
        .with_store(|s| s.local_session_snapshot(&rec.session_id).ok().flatten())
        .and_then(|snap| {
            if snap.title_source == crate::session::TitleSource::None {
                let seed = crate::util::titleize_prompt(&p.prompt);
                if seed.is_empty() {
                    return None;
                }
                state
                    .with_store(|s| {
                        s.seed_title_if_empty(&rec.session_id, snap.turn_id, &seed, now_secs())
                            .ok()
                            .flatten()
                    })
                    .and_then(|updated| {
                        let title = updated.title.trim();
                        (!title.is_empty()).then(|| title.to_string())
                    })
            } else {
                None
            }
        });
    if let Some(room_title) = seeded_title {
        state.status_outbox_notify.notify_waiters();
        spawn_room_name_update(state.clone(), rec.project.clone(), room_title);
    }

    // Publish into the session's CURRENT routing scope — its channel when set
    // (a `channels switch` moved it to a subgroup), else its per-session room.
    // The `is_room` gate above still keys on `project` (the minted room flag is
    // stable across a switch), but the wire `h` tag must follow the channel so
    // the prompt lands where the rest of the session's chat now goes.
    let scope = rec.route_scope().to_string();
    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(op_pubkey.clone(), "operator".to_string()),
        project: scope.clone(),
        body: p.prompt.clone(),
        mentioned_pubkey: None,
    };
    let draft = ChatRecordDraft {
        from_pubkey: op_pubkey,
        from_slug: "operator".to_string(),
        host: state.host.clone(),
        project: scope.clone(),
        body: p.prompt.clone(),
        from_session: rec.session_id.clone(),
        mentioned_session: String::new(),
    };
    // Try once synchronously so local chat history exists when the hook/RPC
    // returns. If relay membership is still converging, fall back to a daemon
    // retry without blocking the editor.
    let publish = publish_chat_checked(state, &chat, &op_keys, &draft);
    match tokio::time::timeout(std::time::Duration::from_secs(3), publish).await {
        Ok(Ok(_)) => {}
        Ok(Err(_)) | Err(_) => {
            spawn_chat_publish_retry(state.clone(), chat, op_keys, draft, "user prompt");
        }
    }

    Ok(serde_json::json!({ "queued": true, "project": scope }))
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
        None,
    )?;
    let id = identity::load_or_create(&config::edge_home(), &rec.agent_slug, now_secs())?;
    let durable_pubkey = id.pubkey_hex();
    // Routing scope: the NIP-29 group this session currently publishes into —
    // its `channel` when set (a `channels switch` moved it to a subgroup), else
    // its per-session room (`project`). All chat routing + the wire `h` tag key
    // on this so a switched session's chat lands in the new room, not the old
    // one it minted at spawn.
    let scope = rec.route_scope().to_string();

    // Mention target: the FIRST inline `@codename` found in the message body,
    // so `chat write "hey @bravo4217"` highlights that session. Only codename-
    // shaped tokens (`<nato-word><digits>`) are recognized — `@` means host in
    // every other tenex-edge identifier, so `@codex` / `@codex@laptop` are NOT
    // mentions. See `idref::extract_mentions`.
    let mention_token: Option<String> = crate::idref::extract_mentions(&p.message)
        .into_iter()
        .next();
    let mention = if let Some(raw) = mention_token {
        let target = state.with_store(|s| resolve_recipient(s, &scope, &state.host, &raw))?;
        let Some(session_id) = target.target_session else {
            anyhow::bail!(
                "mention @{raw} must name a concrete session codename from `tenex-edge who`"
            );
        };
        let same_work_root = state.with_store(|s| {
            let source_root = s.work_root_for_scope(&scope)?;
            let target_root = s.work_root_for_scope(&target.project)?;
            Ok::<bool, anyhow::Error>(source_root == target_root)
        })?;
        if target.project != scope && !same_work_root {
            anyhow::bail!(
                "mention target is in project {:?}, but this chat is for project {:?}",
                target.project,
                scope
            );
        }
        Some((target.pubkey, session_id, target.project))
    } else {
        None
    };
    let mentioned_pubkey = mention.as_ref().map(|(pk, _, _)| pk.clone());
    let mentioned_session = mention.as_ref().map(|(_, sid, _)| sid.clone());
    let publish_scope = mention
        .as_ref()
        .map(|(_, _, project)| project.as_str())
        .unwrap_or(scope.as_str())
        .to_string();

    let chat_signing_keys = state
        .keys_for_session(&rec.session_id)
        .unwrap_or_else(|| id.keys.clone());
    let from_pubkey = chat_signing_keys.public_key().to_hex();

    let chat = ChatMessage {
        from: crate::domain::AgentRef::new(from_pubkey.clone(), rec.agent_slug.clone()),
        project: publish_scope.clone(),
        body: p.message.clone(),
        mentioned_pubkey: mentioned_pubkey.clone(),
    };
    let event_id = state
        .provider
        .publish_checked(&DomainEvent::ChatMessage(chat), &chat_signing_keys)
        .await?;
    let event_id = event_id.to_hex();
    let created_at = now_secs();

    // Local live delivery: relays often don't echo an event back to the same
    // connection that published it, and chat intentionally does not catch up old
    // history. Route now to sessions already alive in the same routing scope
    // (channel when set, else the per-session room) so a `channels switch` is
    // reflected immediately, not only once the relay echoes back.
    let routed = state.with_store(|s| {
        let _ = s.record_chat(&ChatLogRow {
            chat_event_id: event_id.clone(),
            from_pubkey: from_pubkey.clone(),
            from_slug: rec.agent_slug.clone(),
            host: state.host.clone(),
            project: scope.clone(),
            body: p.message.clone(),
            created_at,
            from_session: rec.session_id.clone(),
            mentioned_session: mentioned_session.clone().unwrap_or_default(),
        });
        let mut routed = false;
        for target in s.list_alive_sessions().unwrap_or_default() {
            let is_direct_target = mentioned_session.as_deref() == Some(target.session_id.as_str());
            if !is_direct_target && target.route_scope() != scope {
                continue;
            }
            if target.created_at > created_at {
                continue;
            }
            // Skip sender's own sessions by pubkey.
            if target.agent_pubkey == durable_pubkey || target.agent_pubkey == from_pubkey {
                continue;
            }
            // Preserve local mention routing: if the resolved mention targets
            // this session, mark it as a direct mention in the inbox row.
            let row_mentioned = if is_direct_target {
                target.session_id.clone()
            } else {
                String::new()
            };
            let row_project = if is_direct_target {
                target.route_scope().to_string()
            } else {
                scope.clone()
            };
            let row = ChatInboxRow {
                chat_event_id: event_id.clone(),
                target_session: target.session_id,
                from_pubkey: from_pubkey.clone(),
                from_slug: rec.agent_slug.clone(),
                project: row_project,
                body: p.message.clone(),
                created_at,
                from_session: rec.session_id.clone(),
                mentioned_session: row_mentioned,
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
        project: scope.clone(),
        from: rec.agent_slug,
        from_session: Some(rec.session_id),
        to: mentioned_pubkey
            .as_deref()
            .map(pubkey_short)
            .unwrap_or_else(|| "project-chat".to_string()),
        to_session: mentioned_session.clone(),
        body: p.message.chars().take(200).collect(),
    });

    Ok(serde_json::json!({
        "event_id": event_id,
        "project": publish_scope,
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
    group: Option<String>,
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
///   (no agent/session tag — author identity is the event signer pubkey; kind:0 carries slug)
async fn rpc_propose(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
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
        p.group.as_deref(),
    )
    .ok();
    let cwd = p
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    // Publish into the session's CURRENT routing scope — channel when set
    // (a `channels switch` moved it to a subgroup), else the per-session room.
    // Falls back to the cwd-derived project when no session is live (a bare
    // `tenex-edge publish` from the repo root).
    let project = session_rec
        .as_ref()
        .map(|r| r.route_scope().to_string())
        .unwrap_or_else(|| crate::project::resolve(&cwd).unwrap_or_default());
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

    // Build the Proposal domain event; the wire shape lives in the NIP-29 provider.
    let ev = DomainEvent::Proposal(crate::domain::Proposal {
        agent: crate::domain::AgentRef::new(id.pubkey_hex(), agent_slug.clone()),
        project: project.clone(),
        title: p.title.clone(),
        body: p.body.clone(),
        d: d_tag.clone(),
        // Surface to each owner.
        audience: state.owners.clone(),
    });
    // Checked publish: a NIP-29 relay rejecting the kind:30023 (e.g. the author
    // isn't a member of the project group) used to resolve Ok and report a false
    // "published" — silent data loss. `publish_checked` fails on relay rejection
    // so the CLI exits nonzero with the relay's stated reason.
    // Sign with the selected session key when a live session is present.
    let proposal_signing_keys = session_rec
        .as_ref()
        .and_then(|r| state.keys_for_session(&r.session_id))
        .unwrap_or_else(|| id.keys.clone());
    let event_id = state
        .provider
        .publish_checked(&ev, &proposal_signing_keys)
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

    Ok(serde_json::json!({
        "event_id": eid_hex,
        "d_tag": d_tag,
        "title": p.title,
        "retrievable": retrievable,
    }))
}

struct ResolvedRecipient {
    pubkey: String,
    target_session: Option<String>,
    project: String,
}

/// Resolve a recipient/identifier to a wire pubkey under the CANONICAL scheme:
///   - `agent@host`  → the durable agent on that machine (host always slugified;
///     `@` NEVER means project). The message still goes to `my_project`.
///   - 64-hex / npub → raw pubkey.
///   - a session     → by canonical id, harness alias, id prefix, or codename.
///   - a bare slug   → that agent on the LOCAL host (`slug@<local_host>`).
fn resolve_recipient(
    store: &Store,
    my_project: &str,
    local_host: &str,
    target: &str,
) -> Result<ResolvedRecipient> {
    use crate::idref::{parse_ref, Ref};

    // A session target scopes local delivery and p-tags the session's selected
    // fabric identity: durable by default, transient for collision-fallback
    // duplicates.
    let session_recipient =
        |store: &Store, session_id: String, fallback_pk: String, project: String| {
            ResolvedRecipient {
                pubkey: store
                    .session_pubkey_for_session(&session_id)
                    .unwrap_or(fallback_pk),
                target_session: Some(session_id),
                project,
            }
        };

    match parse_ref(target) {
        // `agent@host` — durable agent on a specific machine.
        Ref::Agent { slug, host } => {
            let pk = store
                .pubkey_for_agent_on_host(&slug, &host)?
                .with_context(|| format!("can't resolve {slug}@{host} (no presence/profile seen yet — try `tenex-edge who`)"))?;
            Ok(ResolvedRecipient {
                pubkey: pk,
                target_session: None,
                project: my_project.to_string(),
            })
        }
        // 64-hex or npub.
        Ref::Pubkey(raw) => {
            let pubkey = nostr_sdk::prelude::PublicKey::parse(&raw)
                .map(|pk| pk.to_hex())
                .unwrap_or(raw);
            Ok(ResolvedRecipient {
                pubkey,
                target_session: None,
                project: my_project.to_string(),
            })
        }
        // A session (id / alias / prefix / codename) OR a bare agent slug.
        Ref::Token(tok) => {
            // 1. Exact canonical id or harness alias.
            if let Some(s) = store.get_session(&tok)? {
                return Ok(session_recipient(
                    store,
                    s.session_id,
                    s.agent_pubkey,
                    s.project,
                ));
            }
            // 2. Session id prefix (peer presence, then own sessions).
            if tok.len() >= 6 {
                if let Some(ps) = store
                    .peer_session_snapshots(None, 0)
                    .unwrap_or_default()
                    .into_iter()
                    .find(|ps| ps.session_id.as_str().starts_with(&tok))
                {
                    return Ok(session_recipient(
                        store,
                        ps.session_id.as_str().to_string(),
                        ps.agent_pubkey,
                        ps.project,
                    ));
                }
                if let Some(s) = store.find_session_by_prefix(&tok)? {
                    return Ok(session_recipient(
                        store,
                        s.session_id,
                        s.agent_pubkey,
                        s.project,
                    ));
                }
            }
            // 3. Session codename (e.g. `bravo4217` from `who`).
            if let Some(found) = find_session_by_codename(store, &tok)? {
                return Ok(session_recipient(
                    store,
                    found.session_id,
                    found.pubkey,
                    found.project,
                ));
            }
            // 4. Bare agent slug → that agent on the LOCAL host.
            if let Some(pk) =
                store.pubkey_for_agent_on_host(&tok, &crate::util::slugify_host(local_host))?
            {
                return Ok(ResolvedRecipient {
                    pubkey: pk,
                    target_session: None,
                    project: my_project.to_string(),
                });
            }
            anyhow::bail!("can't resolve recipient {target:?} (try `tenex-edge who`)")
        }
    }
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

    // Search peer sessions. Production peer presence lives in `peer_session_state`
    // (written by `record_peer_status`), surfaced via `peer_session_snapshots`;
    // the `peer_sessions` table is only populated by tests. The snapshot's
    // `agent_pubkey` is the peer's SESSION pubkey (peer status is session-signed),
    // which is exactly the wire address we want to p-tag.
    if let Ok(peers) = store.peer_session_snapshots(None, 0) {
        for peer in peers {
            if session_codename(peer.session_id.as_str()).to_lowercase() == target_code {
                return Ok(Some(SessionMatch {
                    pubkey: peer.agent_pubkey,
                    session_id: peer.session_id.as_str().to_string(),
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

// ── turn_start / turn_check / turn_end ───────────────────────────────────────

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

    // Emit Turn{working} for the live tail feed. Key on the routing scope
    // (channel when set, else the per-session room) so the tail reflects the
    // room the session actually publishes into after a `channels switch`.
    state.emit_tail(TailEvent::Turn {
        ts: now_secs(),
        project: rec.route_scope().to_string(),
        agent: rec.agent_slug.clone(),
        session: rec.session_id.clone(),
        state: "working".into(),
        elapsed_s: None,
    });

    // Warm the kind:0 profile cache for any pending chat sender we cannot yet
    // name (a human operator / unseen remote agent), so the synchronous context
    // assembly below renders display names rather than raw pubkeys. Resolution
    // (cache→relay) lives in one place: `crate::profile`.
    let to_warm: Vec<String> = state.with_store(|s| {
        let mut v: Vec<String> = Vec::new();
        for r in s.peek_chat(&session).unwrap_or_default() {
            if r.from_slug.is_empty() {
                v.push(r.from_pubkey);
            }
            // Also names referenced by `nostr:npub1…` mentions in the body.
            v.extend(crate::profile::body_mention_pubkeys(&r.body));
        }
        v.sort();
        v.dedup();
        v
    });
    crate::profile::warm(state, &to_warm).await;

    // Assemble via the SHARED cli.rs function so the injected text is byte-identical
    // to the pre-daemon CLI and cannot drift.
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
    #[serde(default)]
    group: Option<String>,
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
        p.group.as_deref(),
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
    /// The agent's turn output (last assistant text), read from the transcript
    /// by the stop hook. Published as kind:9 chat into the session's room.
    #[serde(default)]
    reply: Option<String>,
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
        let (was_working, turn_started_at) =
            state.with_store(|s| s.get_turn_state(&session).unwrap_or((false, 0)));
        state.with_store(|s| {
            // Canonical transition: busy=0, activity cleared, TITLE retained,
            // version bump + status_outbox enqueue. Also clears turn_state.
            s.end_turn(&session, now_secs()).ok();
        });
        state.status_outbox_notify.notify_waiters();

        let rec = state.with_store(|s| s.get_session(&session).ok().flatten());

        // Publish the agent's turn output as kind:9 chat into the session's
        // room (issue #6). Gated to per-session rooms via the local
        // `is_session_room` flag (robust against the relay materializer, unlike
        // `project_meta.parent`), so we never spam the bare project group.
        // Signed by the durable agent key (the room member).
        //
        // Gated on `was_working` so it is IDEMPOTENT against duplicate stop hooks
        // and client retries: `end_turn` above clears the turn, so a second
        // turn_end (e.g. the blocking client retried after its 2s timeout) reads
        // `was_working == false` and never republishes. The publish timeout is
        // kept below that client retry window so the first call resolves first.
        // Best-effort: a relay/parse hiccup must not fail turn-end.
        if was_working {
            if let (Some(rec), Some(reply)) = (rec.as_ref(), p.reply.as_deref()) {
                let reply = reply.trim();
                let is_room = state
                    .with_store(|s| s.is_session_room(&rec.project))
                    .unwrap_or(false);
                // Skip when the reply equals the turn-start baseline — the
                // transcript's last assistant text is unchanged, so this turn
                // produced no new response (e.g. a tool-only turn) and mirroring
                // it would re-publish the PREVIOUS turn's output.
                let baseline =
                    state.with_store(|s| s.get_last_assistant_text_at_turn_start(&session));
                let is_fresh = reply != baseline.trim();
                if !reply.is_empty() && is_room && is_fresh {
                    // Idempotency against duplicate stop hooks / client retries is
                    // provided by the `was_working` gate above (a retry sees the
                    // turn already ended), so the timeout only needs to bound a
                    // hung relay — not race the client retry window.
                    // Timeout kept below the blocking hook client's 2s read
                    // deadline so the daemon fail-opens (and replies) before the
                    // client gives up and retries the stop hook.
                    let publish = publish_agent_reply(state, rec, reply);
                    let debug = std::env::var("TENEX_EDGE_DEBUG").is_ok();
                    match tokio::time::timeout(std::time::Duration::from_millis(1500), publish)
                        .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) if debug => {
                            eprintln!("[daemon] agent reply publish skipped: {e:#}")
                        }
                        Err(_) if debug => eprintln!("[daemon] agent reply publish timed out"),
                        _ => {}
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
            if let Some(rec) = rec.as_ref() {
                state.emit_tail(TailEvent::Turn {
                    ts: now,
                    project: rec.route_scope().to_string(),
                    agent: rec.agent_slug.clone(),
                    session: rec.session_id.clone(),
                    state: "idle".into(),
                    elapsed_s,
                });
            }
            crate::tmux::ring_doorbells(state.clone());
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

// ── local_backend ────────────────────────────────────────────────────────────

/// Return the local daemon's backend pubkey (hex) and host slug so callers can
/// construct `slug@backend` agent specs without guessing the hostname format.
fn rpc_local_backend(state: &Arc<DaemonState>) -> Result<serde_json::Value> {
    let pubkey = state
        .backend_pubkey
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) configured"))?;
    let host_slug = crate::util::slugify_host(&state.host);
    Ok(serde_json::json!({ "pubkey": pubkey, "host_slug": host_slug }))
}

async fn refresh_project_members_cache(state: &Arc<DaemonState>, project: &str) -> bool {
    use crate::fabric::nip29::wire::{kind, KIND_GROUP_MEMBERS};
    use nostr_sdk::prelude::Filter;

    let filter = Filter::new()
        .kind(kind(KIND_GROUP_MEMBERS))
        .identifier(project)
        .limit(5);
    let Ok(events) = state.transport.fetch(filter, Duration::from_secs(5)).await else {
        return false;
    };
    let Some(ev) = events.iter().max_by_key(|e| e.created_at.as_secs()) else {
        return false;
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
    true
}

async fn wait_for_project_member_cache(
    state: &Arc<DaemonState>,
    project: &str,
    pubkey: &str,
    present: bool,
) -> bool {
    for _ in 0..20 {
        let refreshed = refresh_project_members_cache(state, project).await;
        let has = state.with_store(|s| s.is_group_member(project, pubkey).unwrap_or(false));
        if refreshed && has == present {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

// ── statusline ───────────────────────────────────────────────────────────────

/// How long a drained mention keeps showing in the statusline RPC snapshot as
/// "recently consumed" before disappearing. The renderer no longer displays the
/// inbox segment, but callers still consume `pending`/`recent` from this RPC.
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
    // Session ID is the only locator needed. Fail open (empty bar) when it is
    // absent or stale — the next session_start reassert will refresh @te_session
    // on the tmux session and the bar recovers on the next status-interval tick.
    let session_id = p.session.as_deref().filter(|s| !s.is_empty());
    let rec = match session_id {
        Some(id) => match state.with_store(|s| s.get_session(id)).ok().flatten() {
            Some(r) => r,
            None => {
                // ID present but not in the DB — stale @te_session (e.g. after a
                // DB restore). Return a visible error instead of an empty bar.
                return Ok(serde_json::json!({ "error": "stale" }));
            }
        },
        None => return Ok(serde_json::json!({})),
    };
    let now = now_secs();
    let host = state.host.clone();
    // Routing scope (channel when set, else the per-session room) — the member
    // count and is_member check key on it so a `channels switch` is reflected in
    // the statusline without restarting.
    let scope = rec.route_scope().to_string();
    state.with_store(|s| {
        let member_count = s.count_group_members(&scope).unwrap_or(0);
        let is_member = s.is_group_member(&scope, &rec.agent_pubkey).unwrap_or(true);
        // Read busy + title + live activity from the canonical aggregate via
        // the SHARED projection (derive_status), so the statusline agrees with
        // `who`/turn-deltas. Pure read: no drains, no touches. The statusline
        // shows the activity line (the live "doing now" signal from kind:30315),
        // not the persistent title (the channel title segment carries that).
        let (working, title, activity) = s
            .local_session_snapshot(&rec.session_id)
            .ok()
            .flatten()
            .map(|snap| {
                let d = derive_status(&snap, now);
                (d.busy, d.title, d.activity)
            })
            .unwrap_or((false, String::new(), String::new()));
        // `channel_title` is the display name of the routing scope (channel or
        // per-session room) from the relay-authored kind:39000 metadata cache
        // (== the channel's title on the relay == what the relay renders as the
        // room's name). Falls back to the distilled session title when the
        // session is in its own per-session room (issue #6: the room is renamed
        // to the distilled title via kind:9002 edit-metadata; the local cache
        // may lag by one refresh).
        let mut channel_title = s.group_display_name(&scope).unwrap_or_default();
        if channel_title.is_empty() {
            channel_title = title.clone();
        }
        // `work_root` is the parent project a per-session room or task channel
        // hangs under, or the project itself for an ordinary project session.
        // This is the "Project" line in `who`, surfaced as `project-name`.
        let work_root = s
            .work_root_for_scope(&scope)
            .unwrap_or_else(|_| rec.project.clone());
        let pending_chat = s.peek_chat_mentions(&rec.session_id).unwrap_or_default();
        let recent_since = now.saturating_sub(STATUSLINE_RECENT_SECS);
        let recent_chat = s
            .list_recently_delivered_chat_mentions(&rec.session_id, recent_since)
            .unwrap_or_default();
        let mut pending_json = chat_rows_to_json(&pending_chat);
        sort_message_json(&mut pending_json);
        let mut recent_json = chat_rows_to_json(&recent_chat);
        sort_message_json(&mut recent_json);
        let distill_error = s
            .get_recent_session_error(&rec.session_id, now.saturating_sub(DISTILL_ERROR_TTL_SECS))
            .ok()
            .flatten();
        Ok(serde_json::json!({
            "agent": rec.agent_slug,
            "host": host,
            "session_id": rec.session_id,
            "work_root": work_root,
            "channel": scope,
            "channel_title": channel_title,
            "member_count": member_count,
            "is_member": is_member,
            "working": working,
            "title": title,
            "activity": activity,
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
        None,
        false,
    )?;
    let now = now_secs();
    let host = state.host.clone();
    // Routing scope: channel when set (a `channels switch` moved the session),
    // else the per-session room. `whoami` answers "which agent am I on the
    // fabric" — the scope it currently publishes into is the relevant one, and
    // the is_member check must key on it so a switched session doesn't report
    // a stale membership in the room it minted at spawn.
    let scope = rec.route_scope().to_string();
    state.with_store(|s| {
        let pubkey = s
            .session_pubkey_for_session(&rec.session_id)
            .unwrap_or_else(|| rec.agent_pubkey.clone());
        let npub = {
            use nostr_sdk::prelude::ToBech32;
            nostr_sdk::PublicKey::from_hex(&pubkey)
                .ok()
                .and_then(|pk| pk.to_bech32().ok())
        };
        let is_member = s.is_group_member(&scope, &pubkey).unwrap_or(true);
        let (working, status) = s
            .local_session_snapshot(&rec.session_id)
            .ok()
            .flatten()
            .map(|snap| {
                let d = derive_status(&snap, now);
                (d.busy, d.title)
            })
            .unwrap_or((false, String::new()));
        let pending = s
            .peek_chat_mentions(&rec.session_id)
            .unwrap_or_default()
            .len();
        Ok(serde_json::json!({
            "agent": rec.agent_slug,
            "session_id": rec.session_id,
            "codename": crate::util::session_codename(&rec.session_id),
            "project": scope,
            "host": host,
            "rel_cwd": rec.rel_cwd,
            "pubkey": pubkey,
            "npub": npub,
            "is_member": is_member,
            "working": working,
            "status": status,
            "pending": pending,
            "created_at": rec.created_at,
        }))
    })
}

// ── project_add ──────────────────────────────────────────────────────────────

fn log_nip29_role_decision(group: &str, pubkey: &str, role: &str, reason: &str) {
    eprintln!(
        "[daemon] nip29-role-decision group={group} target={} role={role} reason={reason}",
        crate::util::pubkey_short(pubkey)
    );
}

fn rpc_debug_outbox(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        #[serde(default = "default_debug_outbox_limit")]
        limit: u64,
    }
    let p: P = serde_json::from_value(params.clone()).unwrap_or(P {
        limit: default_debug_outbox_limit(),
    });
    let rows = state.with_store(|s| s.list_status_outbox_debug(p.limit))?;
    let rows = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "session_id": r.session_id,
                "state_version": r.state_version,
                "publish_state": r.publish_state,
                "retries": r.retries,
                "native_event_id": r.native_event_id,
                "last_error": r.last_error,
                "enqueued_at": r.enqueued_at,
                "agent_slug": r.agent_slug,
                "project": r.project,
                "title": r.title,
                "activity": r.activity,
                "busy": r.busy,
            })
        })
        .collect::<Vec<_>>();
    Ok(serde_json::json!({ "rows": rows }))
}

fn default_debug_outbox_limit() -> u64 {
    50
}

// ── project create-group (NIP-29 subgroup task rooms, issue #3) ───────────────

/// Create a NIP-29 SUBGROUP under `parent`, copy the parent's trusted admin set
/// down, and publish ONE kind:9 orchestration event asking the named backends to
/// add their agent roles. Provision-only: this command does not spawn harnesses.
/// Each backend (including this one) reacts to the orchestration event via the
/// `handle_orchestration` listener, which is what makes cross-device auto-start
/// work; we invoke it locally here too since relays don't reliably echo to the
/// publishing connection.
/// Mint (idempotently) a per-session room: a NIP-29 subgroup `room_h` parented
/// under the work-root `parent` project that a human-initiated session lives in
/// (issue #6). Ensures the parent group exists, creates+locks the child, waits
/// for the relay to reflect our admin status (so the dependent member-add
/// applies on the first try), owns+subscribes locally, and adds the agent's
/// durable pubkey as a member.
///
/// Best-effort and fail-open: a relay rejection leaves the session running
/// (it just won't have a relay-backed room). Returns true when the room was
/// created/confirmed on the relay.
async fn ensure_session_room(
    state: &Arc<DaemonState>,
    room_h: &str,
    name: &str,
    parent: &str,
    agent_pubkey: &str,
) -> bool {
    use nostr_sdk::prelude::Keys;
    let Some(nsec) = state.cfg.management_nsec() else {
        return false;
    };
    let Ok(mgmt_keys) = Keys::parse(nsec) else {
        return false;
    };
    let mgmt_pk = mgmt_keys.public_key().to_hex();

    // The parent project group must exist before a child can be parented under it.
    state.provider.open_project(parent, agent_pubkey).await;

    // Create + lock the child with its parent relationship. Session room ids are
    // deterministic, so a daemon restart or repeated test run may find the room
    // already present on the relay. Treat that as success and rehydrate local
    // ownership metadata instead of failing before `mark_group_owned`.
    log_nip29_role_decision(
        room_h,
        &mgmt_pk,
        "admin",
        "ensure_session_room subgroup create signer becomes relay admin",
    );
    let (exists_before, _, _) = state.provider.fetch_group_state(room_h).await;
    let created_or_exists = exists_before
        || state
            .provider
            .nip29_create_subgroup(room_h, name, parent)
            .await
        || state.provider.fetch_group_state(room_h).await.0;
    if !created_or_exists {
        eprintln!("[daemon] session room {room_h} create/lock not accepted (best-effort)");
        return false;
    }

    // Own + subscribe and persist the room's hierarchy (name + parent) into the
    // local read-model IMMEDIATELY after the create succeeds — BEFORE the slow
    // admin-reflection poll below — so that even if the poll/member-add stall or
    // the caller's mint timeout fires, the room is still locally recognized (the
    // `group_parent`/`is_session_room` gates for prompt+reply mirroring depend on
    // it) and `groups list` sees it. Eager because the relay's kind:39000 echo
    // arrives later.
    state.with_store(|s| {
        s.mark_group_owned(room_h, now_secs()).ok();
        s.mark_session_room(room_h, parent, now_secs()).ok();
        s.upsert_group_metadata(room_h, name, parent, now_secs())
            .ok();
    });
    let latest_title =
        state.with_store(|s| s.latest_session_title_for_project(room_h).ok().flatten());
    if let Some(title) = latest_title {
        apply_room_name_update(state, room_h, &title).await;
    }
    let _ = ensure_subscription(state, room_h).await;

    // Wait for the relay to reflect OUR admin status (recorded while processing
    // the 9007 create) before issuing the dependent member-add, which the relay
    // validates against the author's admin role at apply-time. Bounded to a few
    // attempts (~3s worst case): this runs on the session-birth path, so it must
    // not stall the first prompt. The common case resolves on attempt 0-1.
    for attempt in 0..6u32 {
        let roles = state.provider.fetch_group_roles(room_h).await;
        if roles.get(&mgmt_pk).map(String::as_str) == Some("admin") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            250 * (attempt as u64 + 1).min(3),
        ))
        .await;
    }

    // Add the agent's durable pubkey as a member of its own room. Relay admin
    // reflection can lag the subgroup create, so retry without marking local
    // membership until the relay accepts the add.
    log_nip29_role_decision(
        room_h,
        agent_pubkey,
        "member",
        "ensure_session_room adds owning agent durable pubkey",
    );
    for attempt in 0..12u32 {
        let outcome = state
            .provider
            .nip29_add_member_outcome(room_h, agent_pubkey)
            .await;
        if outcome.is_applied() {
            if wait_for_project_member_cache(state, room_h, agent_pubkey, true).await {
                break;
            }
        }
        if outcome.is_rejected() {
            eprintln!(
                "[daemon] session-room member-add for {} in {room_h} rejected permanently",
                crate::util::pubkey_short(agent_pubkey)
            );
            break;
        }
        tokio::time::sleep(nip29_retry_delay(attempt)).await;
    }
    // Copy parent project admins + whitelist + backend pubkey to the session
    // room — same invariant as channels_create: authority flows downward so the
    // operator can manage every room without being added to it explicitly.
    let parent_roles = state.provider.fetch_group_roles(parent).await;
    let mut admin_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (pk, role) in &parent_roles {
        if role == "admin" {
            log_nip29_role_decision(
                room_h,
                pk,
                "admin",
                "ensure_session_room copied parent admin",
            );
            admin_set.insert(pk.clone());
        }
    }
    for pk in &state.cfg.whitelisted_pubkeys {
        log_nip29_role_decision(
            room_h,
            pk,
            "admin",
            "ensure_session_room whitelisted_pubkeys grant",
        );
        admin_set.insert(pk.clone());
    }
    if let Some(bp) = state.backend_pubkey() {
        log_nip29_role_decision(
            room_h,
            bp,
            "admin",
            "ensure_session_room backend pubkey grant",
        );
        admin_set.insert(bp.to_string());
    }
    for pk in &admin_set {
        if state.provider.nip29_add_admin(room_h, pk).await {
            state.with_store(|s| {
                s.upsert_group_member(room_h, pk, "admin", now_secs()).ok();
            });
        }
    }
    true
}

async fn rpc_channels_create(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    use crate::fabric::nip29::orchestration::{build_add_agents_event, AddTarget};
    use nostr_sdk::prelude::Keys;

    #[derive(serde::Deserialize)]
    struct AgentSpec {
        slug: String,
        backend: String,
    }
    #[derive(serde::Deserialize)]
    struct P {
        parent: String,
        name: String,
        #[serde(default)]
        agents: Vec<AgentSpec>,
        #[serde(default)]
        brief: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_create params")?;
    if p.agents.is_empty() {
        anyhow::bail!("at least one agent (slug@backend) is required");
    }

    // Relay subgroup-support verification is handled by a separate workstream;
    // call its gate here when it lands. For now we proceed and fail loudly below
    // if the relay rejects the subgroup create/lock.

    let nsec = state
        .cfg
        .management_nsec()
        .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) set"))?;
    let mgmt_keys = Keys::parse(nsec).context("parsing signing key")?;
    let mgmt_pk = mgmt_keys.public_key().to_hex();

    // Short child id; hierarchy lives in metadata, not the id.
    let child_h = crate::util::child_group_id(&p.name);

    // Resolve each backend token to a hex pubkey. Accepts explicit
    // pubkey/npub/NIP-05 *and* host slugs as shown by `tenex-edge who`.
    let mut adds: Vec<AddTarget> = Vec::with_capacity(p.agents.len());
    for a in &p.agents {
        let backend_pubkey = resolve_backend_pubkey(state, &a.backend)
            .await
            .with_context(|| format!("resolving backend {:?}", a.backend))?;
        eprintln!(
            "[daemon] nip29-role-decision channel={} requested_agent={} backend={} backend_pubkey={} role=member reason=channels_create orchestration target; backend may be admin but spawned agent must be member",
            child_h,
            a.slug,
            a.backend,
            crate::util::pubkey_short(&backend_pubkey)
        );
        adds.push(AddTarget {
            backend_pubkey,
            slug: a.slug.clone(),
        });
    }

    // Create + lock the child with its parent relationship. Fail loudly: a
    // create-group that didn't actually create a group must not look successful.
    log_nip29_role_decision(
        &child_h,
        &mgmt_pk,
        "admin",
        "channels_create subgroup create signer becomes relay admin",
    );
    let created = state
        .provider
        .nip29_create_subgroup(&child_h, &p.name, &p.parent)
        .await;
    if !created {
        anyhow::bail!(
            "relay did not accept subgroup create/lock for {child_h} (parent {}); \
             does the relay support NIP-29 subgroups and is the signing key an admin?",
            p.parent
        );
    }

    // Gate on the relay reflecting OUR OWN admin status before issuing any
    // dependent management events. The relay records the creator as admin while
    // processing the 9007, but a put-admin / put-user we fire immediately can
    // race that and be dropped (the relay validates the author's admin role at
    // apply-time). Poll the child's 39001 until the signing key is admin; every
    // subsequent grant/add then applies on the first try.
    for attempt in 0..20u32 {
        let roles = state.provider.fetch_group_roles(&child_h).await;
        if roles.get(&mgmt_pk).map(String::as_str) == Some("admin") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            300 * (attempt as u64 + 1).min(4),
        ))
        .await;
    }

    // Admin set for the child: copy ALL parent admins (our invariant is that
    // agents/sessions are NEVER admins, so the parent admin set is exactly the
    // trusted human/backend/operator/friend pubkeys), plus this daemon's own
    // identity, the operator pubkey, and the configured whitelist — so the local
    // backend can manage the child and authority carries downward.
    let mut admin_set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let parent_roles = state.provider.fetch_group_roles(&p.parent).await;
    eprintln!(
        "[daemon] nip29-role-decision channel={child_h} parent={} parent_role_count={} whitelist_count={} backend_pubkey={} reason=channels_create building child admin set",
        p.parent,
        parent_roles.len(),
        state.cfg.whitelisted_pubkeys.len(),
        state
            .backend_pubkey()
            .map(crate::util::pubkey_short)
            .unwrap_or_else(|| "none".to_string())
    );
    for (pk, role) in &parent_roles {
        if role == "admin" {
            log_nip29_role_decision(
                &child_h,
                pk,
                "admin",
                "channels_create copied parent admin from relay 39001",
            );
            admin_set.insert(pk.clone());
        } else {
            log_nip29_role_decision(
                &child_h,
                pk,
                role,
                "channels_create parent role not copied to child admin set",
            );
        }
    }
    for pk in &state.cfg.whitelisted_pubkeys {
        log_nip29_role_decision(
            &child_h,
            pk,
            "admin",
            "channels_create configured whitelisted_pubkeys grant",
        );
        admin_set.insert(pk.clone());
    }
    if let Some(bp) = state.backend_pubkey() {
        log_nip29_role_decision(
            &child_h,
            bp,
            "admin",
            "channels_create backend tenexPrivateKey pubkey grant",
        );
        admin_set.insert(bp.to_string());
    }
    // Grant each admin and CONFIRM it landed in the relay's 39001 roster. Like
    // member-adds, the relay acks a put-admin on receipt but only APPLIES it once
    // the author's own admin status (from the 9007 create we just published) has
    // propagated — so the first grant for a second admin can silently no-op.
    // Trust-but-verify: re-issue + read back the role map a few times.
    let mut granted: Vec<String> = Vec::new();
    for pk in &admin_set {
        let mut confirmed = false;
        for attempt in 0..6u32 {
            let outcome = state.provider.nip29_add_admin_outcome(&child_h, pk).await;
            let (_, roles, _) = state.provider.fetch_group_state(&child_h).await;
            // Confirm via the published 39001 roster OR a benign re-issue (attempt
            // > 0 accepted as "already a member" → the relay's authoritative state
            // already grants the role). Same stale-replaceable defense as the
            // member-add loop below: a same-second created_at collision can freeze
            // 39001, so the roster readback alone can deadlock the grant.
            if roles.get(pk).map(String::as_str) == Some("admin")
                || (attempt > 0 && outcome.is_applied())
            {
                confirmed = true;
                break;
            }
            if outcome.is_rejected() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * (attempt as u64 + 1))).await;
        }
        if confirmed {
            state.with_store(|s| {
                s.upsert_group_member(&child_h, pk, "admin", now_secs())
                    .ok();
            });
            granted.push(pk.clone());
        } else {
            eprintln!(
                "[daemon] channels_create: admin grant for {} in {child_h} not confirmed on the relay",
                crate::util::pubkey_short(pk)
            );
        }
    }

    // Own + subscribe to the child so we receive its relay-authored state.
    state.with_store(|s| {
        s.mark_group_owned(&child_h, now_secs()).ok();
    });
    let _ = ensure_subscription(state, &child_h).await;

    // Auto-join the creator: when this command is run from an agent session, add
    // that agent's durable pubkey as a member of the room it just made, so the
    // creator participates in its own subgroup. Best-effort; skipped for a bare
    // operator invocation with no resolvable session.
    let creator: Option<String> = resolve_session(
        state,
        None,
        params.get("env_session").and_then(|v| v.as_str()),
        params.get("cwd").and_then(|v| v.as_str()),
        params.get("agent").and_then(|v| v.as_str()),
        None,
    )
    .ok()
    .map(|rec| rec.agent_pubkey);
    if let Some(ref pk) = creator {
        log_nip29_role_decision(
            &child_h,
            pk,
            "member",
            "channels_create auto-join creator agent",
        );
        if state.provider.nip29_add_member(&child_h, pk).await {
            state.with_store(|s| {
                s.upsert_group_member(&child_h, pk, "member", now_secs())
                    .ok();
            });
        }
    }

    // Build + publish ONE kind:9 orchestration event into the parent (the
    // coordination group). The child id rides in an `h-target` tag.
    let prose = if p.brief.trim().is_empty() {
        generate_orchestration_prose(&adds)
    } else {
        p.brief.clone()
    };
    let builder = build_add_agents_event(&p.parent, &child_h, &adds, &prose)?;
    let signed = state.transport.sign(builder, &mgmt_keys).await?;
    let orchestration_event_id = signed.id.to_hex();
    // Checked publish: the bare `publish_event` resolves `Ok` even when every
    // relay rejected the kind:9 (NIP-29 `blocked` / rate-limited), so reporting
    // `orchestration_event_id` off it would advertise a channel whose
    // orchestration event was silently dropped — backends would never receive
    // the add-agents directive. `publish_event_checked` turns a relay rejection
    // into a hard error so `channels_create` fails loudly instead of lying
    // about success.
    state.transport.publish_event_checked(&signed).await?;

    // Local fast-path: relays don't reliably echo to the publishing connection,
    // so drive the same listener directly for roles targeted at THIS backend.
    // Idempotency is enforced inside handle_orchestration via processed_orchestration.
    if let Some(op) = crate::fabric::nip29::orchestration::parse_orchestration(&signed) {
        handle_orchestration(state, &signed, op).await;
    }

    Ok(serde_json::json!({
        "child_h": child_h,
        "display_path": format!("{} > {}", p.parent, p.name),
        "admins": granted,
        "creator": creator.unwrap_or_default(),
        "orchestration_event_id": orchestration_event_id,
    }))
}

/// `channels list`: render the subgroup tree under `project` from LOCAL daemon
/// state (materialized kind:39000 metadata) — no relay round-trip. Returns the
/// rooms in depth-first order, each with a `depth` (the project root is depth 0
/// and not included; its direct children are depth 1) so the CLI can indent.
fn rpc_channels_list(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        project: String,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_list params")?;

    // (group_id, about, name, parent) for every group the daemon knows about.
    let rows = state.with_store(|s| s.list_group_metadata())?;
    // parent id -> children (id, display name). Sorted for stable output.
    let mut children: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for (id, about, name, parent) in &rows {
        if parent.is_empty() {
            continue;
        }
        let display = if name.is_empty() {
            about.clone()
        } else {
            name.clone()
        };
        children
            .entry(parent.clone())
            .or_default()
            .push((id.clone(), display));
    }
    for v in children.values_mut() {
        v.sort();
    }

    let rooms = preorder_rooms(&children, &p.project);
    Ok(serde_json::json!({ "project": p.project, "rooms": rooms }))
}

/// Pre-order DFS flatten of the subgroup tree rooted at `root` into
/// `{child_h, name, depth}` JSON (root excluded, its children at depth 0).
fn preorder_rooms(
    children: &std::collections::BTreeMap<String, Vec<(String, String)>>,
    root: &str,
) -> Vec<serde_json::Value> {
    fn walk(
        children: &std::collections::BTreeMap<String, Vec<(String, String)>>,
        node: &str,
        depth: usize,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<serde_json::Value>,
    ) {
        if let Some(kids) = children.get(node) {
            for (child_id, name) in kids {
                if !seen.insert(child_id.clone()) {
                    continue;
                }
                out.push(serde_json::json!({
                    "child_h": child_id,
                    "name": name,
                    "depth": depth,
                }));
                walk(children, child_id, depth + 1, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    seen.insert(root.to_string());
    walk(children, root, 0, &mut seen, &mut out);
    out
}

/// `channels_switch`: move a running session to a different NIP-29 subgroup
/// without restarting. Writes `sessions.channel`, re-points `session_state.
/// project` at the new scope (so the status drainer, `who`/`statusline`
/// scoping, and `status_delta_since` all key on it), bumps the state version,
/// and enqueues a status_outbox row so a fresh kind:30315 publishes into the
/// new room. All fabric publishing (chat/mentions/proposals), local chat
/// routing, and turn-context deltas follow the new scope via `route_scope()`.
/// Fails if the channel does not exist in local state.
async fn rpc_channels_switch(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct P {
        channel: String,
        #[serde(default)]
        env_session: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).context("channels_switch params")?;
    if p.channel.trim().is_empty() {
        anyhow::bail!("channel h must not be empty");
    }
    let env_session = p
        .env_session
        .as_deref()
        .filter(|s| !s.is_empty())
        .context("channels switch must be run from within a tenex-edge agent session (TENEX_EDGE_SESSION is not set)")?;
    let rec = resolve_session(state, None, Some(env_session), None, None, None)?;
    let new_channel = p.channel.clone();
    // Validate the channel exists in local state before switching.
    let exists: bool =
        state.with_store(|s| Ok::<bool, anyhow::Error>(s.channel_exists(&new_channel)))?;
    if !exists {
        anyhow::bail!("channel {:?} does not exist", new_channel);
    }
    refresh_project_members_cache(state, &new_channel).await;
    let is_member = state.with_store(|s| {
        Ok::<bool, anyhow::Error>(
            s.is_group_member(&new_channel, &rec.agent_pubkey)
                .unwrap_or(false),
        )
    })?;
    if !is_member {
        anyhow::bail!(
            "agent {} is not a member of channel {:?}",
            rec.agent_slug,
            new_channel
        );
    }
    ensure_subscription(state, &new_channel).await?;
    let prev_channel = rec.channel.clone();
    // Apply the switch in ONE store transaction: update `sessions.channel`,
    // move `session_state.project` to the new scope (so the status drainer, who
    // snapshot, and status_delta all key on it), bump the version, and enqueue
    // a status_outbox row so a fresh kind:30315 publishes into the new room.
    // Without this, `channels switch` only flipped a column and the session
    // kept routing into its old per-session room.
    state.with_store(|s| s.set_session_channel(&rec.session_id, &new_channel, now_secs()))?;
    // Nudge the drainer so the scope-changed status publishes immediately
    // rather than waiting for the next heartbeat tick. The kind:30315 it
    // publishes carries the new `h` tag, so peers in the channel see the
    // session's presence without a separate profile push.
    state.status_outbox_notify.notify_waiters();
    Ok(serde_json::json!({
        "session_id": rec.session_id,
        "prev_channel": prev_channel,
        "channel": new_channel,
    }))
}

/// Human-readable summary of the add-agents request, grouped per backend, e.g.
/// "@<edge1>: add research-lead. @<edge2>: add implementation-lead and test1."
/// Advisory only — receivers act on the structured tags, never this prose.
fn generate_orchestration_prose(adds: &[crate::fabric::nip29::orchestration::AddTarget]) -> String {
    use std::collections::BTreeMap;
    let mut by_backend: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for a in adds {
        by_backend
            .entry(a.backend_pubkey.as_str())
            .or_default()
            .push(a.slug.as_str());
    }
    let mut parts: Vec<String> = Vec::new();
    for (backend, slugs) in by_backend {
        parts.push(format!(
            "@{}: add {}.",
            crate::util::pubkey_short(backend),
            slugs.join(" and ")
        ));
    }
    parts.join(" ")
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
        is_backend: false,
    });
    let event_id = state.provider.publish(&ev, &id.keys).await?;

    Ok(serde_json::json!({
        "slug": p.slug,
        "pubkey": id.pubkey_hex(),
        "event_id": event_id.to_hex(),
    }))
}

/// Resolve a backend token (from `slug@<token>`) to a hex pubkey.
/// Accepts: explicit hex pubkey / npub / NIP-05 (via `resolve_pubkey_hex`),
/// OR a host slug as shown by `who` (e.g. `laptop`).  The host-slug path
/// checks the local machine first, then the state store for remote peers.
async fn resolve_backend_pubkey(state: &Arc<DaemonState>, token: &str) -> Result<String> {
    // Fast path: explicit pubkey / npub / NIP-05.
    if let Ok(pk) = resolve_pubkey_hex(token).await {
        return Ok(pk);
    }

    // Host-slug path: `who` renders backends as `slugify_host(backendName)`.
    let local_slug = crate::util::slugify_host(&state.host);
    if token == local_slug {
        return state.backend_pubkey.clone().ok_or_else(|| {
            anyhow::anyhow!(
                "backend token {token:?} matches local host but no signing key is configured"
            )
        });
    }

    // Remote peer: scan profiles / peer_sessions.
    if let Some(pk) = state.with_store(|s| s.pubkey_for_host_slug(token)) {
        return Ok(pk);
    }

    anyhow::bail!(
        "cannot resolve backend {token:?}: not a pubkey/npub/NIP-05 and no known peer with that host slug"
    )
}

async fn resolve_project_member_pubkey_hex(input: &str) -> Result<String> {
    let edge_home = config::edge_home();
    if let Some(agent) = identity::list_local_agent_details(&edge_home)
        .into_iter()
        .find(|agent| agent.slug == input)
    {
        return Ok(agent.pubkey);
    }

    resolve_pubkey_hex(input).await.with_context(|| {
        format!("resolving {input:?} as a local agent slug, pubkey, npub, or NIP-05 address")
    })
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

// ── chat read (backfill + optional live stream) ───────────────────────────────

#[derive(serde::Deserialize, Default)]
struct ChatReadParams {
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    env_session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    group: Option<String>,
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
    let scope = match p.channel.filter(|s| !s.trim().is_empty()) {
        Some(channel) => channel,
        None => resolve_session_inner(
            state,
            p.session.as_deref(),
            p.env_session.as_deref(),
            p.cwd.as_deref(),
            p.agent.as_deref(),
            p.group.as_deref(),
            false,
        )?
        .route_scope()
        .to_string(),
    };
    let since = p.since.unwrap_or(0);
    let offset = p.offset.unwrap_or(0);

    let _ = ensure_subscription(state, &scope).await;
    let mut rx = if p.live {
        Some(state.tail_subscribe())
    } else {
        None
    };
    let live_started_at = now_secs();

    let rows = state.with_store(|s| {
        let mut scopes = vec![scope.clone()];
        if s.session_room_parent(&scope).ok().flatten().is_none() {
            scopes.extend(s.session_rooms_under(&scope).unwrap_or_default());
        }
        let mut rows: Vec<ChatLogRow> = scopes
            .iter()
            .flat_map(|scope| {
                s.list_chat_messages(scope, since, None, 0, false)
                    .unwrap_or_default()
            })
            .collect();
        rows.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then_with(|| a.chat_event_id.cmp(&b.chat_event_id))
        });
        if p.tail {
            let limit = p.limit.unwrap_or(10) as usize;
            let start = rows
                .len()
                .saturating_sub(limit.saturating_add(offset as usize));
            let end = rows.len().saturating_sub(offset as usize);
            rows = rows.get(start..end).unwrap_or(&[]).to_vec();
        } else {
            let start = offset as usize;
            let end = p
                .limit
                .map(|limit| start.saturating_add(limit as usize))
                .unwrap_or(rows.len())
                .min(rows.len());
            rows = rows.get(start..end).unwrap_or(&[]).to_vec();
        }
        rows
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
                ..
            }) if ev_project == scope => {
                let rows = state.with_store(|s| {
                    s.list_chat_messages(&scope, cursor, None, 0, false)
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
                if tail_event_matches_project(&ev, project.as_deref())
                    && ev.ts() >= since
                    && write_json(writer, &Response::item(id, serde_json::to_value(&ev)?))
                        .await
                        .is_err()
                {
                    break;
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

    // ── Recent chat lines from chat_messages ───────────────────────────────────
    let raw_msgs: Vec<(u64, String, String, String, Option<String>)> = state.with_store(|s| {
        s.recent_chat_for_backfill(project, since, limit)
            .unwrap_or_default()
    });

    for (ts, body, author_pubkey, proj, author_session) in raw_msgs {
        // Resolve slug from pubkey.
        let from_slug = state
            .with_store(|s| s.resolve_slug_for_pubkey(&author_pubkey))
            .ok()
            .flatten()
            .unwrap_or_else(|| pubkey_short(&author_pubkey));
        events.push(TailEvent::Msg {
            ts,
            project: proj,
            from: from_slug,
            from_session: author_session,
            to: String::new(), // backfill: recipient not stored inline
            to_session: None,
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
            let tracked_keys: Vec<(String, String)> = {
                let map = state.peer_sessions.lock().unwrap();
                // We'll emit Leave for (pubkey, project) pairs in our map that are
                // no longer live in the store. Cross-reference below.
                map.keys().cloned().collect()
            };

            // Which (pubkey, project) pairs are still live in the store.
            let still_alive: std::collections::HashSet<(String, String)> = state
                .with_store(|s| s.list_peer_sessions(None, before).unwrap_or_default())
                .into_iter()
                .map(|p| (p.pubkey, p.project))
                .collect();

            // Prune from DB.
            state.with_store(|s| {
                let _ = s.prune_peer_sessions(before);
            });

            // Emit Leave for pairs that were in our map but are now expired.
            let to_leave: Vec<((String, String), PeerTracked)> = {
                let mut map = state.peer_sessions.lock().unwrap();
                let expired: Vec<(String, String)> = tracked_keys
                    .into_iter()
                    .filter(|k| !still_alive.contains(k))
                    .collect();
                let mut leaves = Vec::new();
                for key in expired {
                    if let Some(tracked) = map.remove(&key) {
                        leaves.push((key, tracked));
                    }
                }
                leaves
            };
            for ((pubkey, _project), tracked) in to_leave {
                let online_s = now.saturating_sub(tracked.first_seen);
                state.emit_tail(TailEvent::Leave {
                    ts: now,
                    project: tracked.project,
                    agent: tracked.slug,
                    host: tracked.host,
                    session: pubkey,
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
fn status_from_snapshot(snap: &SessionSnapshot, now: u64) -> crate::domain::Status {
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
    }
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
                // Sign with the selected session identity so a heartbeat cannot
                // collapse two duplicate sessions back onto the durable author.
                let keys = match state
                    .keys_for_session(snap.session_id.as_str())
                    .or_else(|| state.keys_for(&snap.agent_pubkey))
                {
                    Some(k) => k,
                    None => continue,
                };
                let status = status_from_snapshot(&snap, now);
                if let Ok(eid) = state.provider.set_status(&status, &keys).await {
                    let signer_pubkey = keys.public_key().to_hex();
                    state.with_store(|s| {
                        s.confirm_local_presence(&snap, &signer_pubkey, &eid.to_hex(), now)
                            .ok();
                    });
                }
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
                    // Sign with the session key for transient duplicates; fall
                    // back to the durable agent key for the default signer.
                    let keys = match state
                        .keys_for_session(&item.session_id)
                        .or_else(|| state.keys_for(&item.snapshot.agent_pubkey))
                    {
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
                    let status = status_from_snapshot(&item.snapshot, now);
                    match state.provider.set_status(&status, &keys).await {
                        Ok(eid) => {
                            let signer_pubkey = keys.public_key().to_hex();
                            state.with_store(|s| {
                                s.mark_status_published(
                                    &item.session_id,
                                    item.state_version,
                                    &eid.to_hex(),
                                )
                                .ok();
                                s.confirm_local_presence(
                                    &item.snapshot,
                                    &signer_pubkey,
                                    &eid.to_hex(),
                                    now,
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
    let proj = project.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(params, provider, store, cancel).await;
        if let Err(e) = res {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] session {sid} task error: {e:#}");
            }
        }
        // Engine self-exit path: remove a transient duplicate signer from the
        // NIP-29 group. The Mutex pop is atomic: if rpc_session_end already
        // removed the key, this finds None and avoids a duplicate publish.
        {
            let maybe_key = st.release_session_signer(&sid, &pubkey, &proj);
            if let Some(sk) = maybe_key {
                let session_pubkey = sk.public_key().to_hex();
                st.provider
                    .nip29_remove_member(&proj, &session_pubkey)
                    .await;
                st.with_store(|s| {
                    s.remove_group_member(&proj, &session_pubkey).ok();
                });
            }
        }
        // Clear the DB routing row regardless of whether the in-memory key was
        // still present (graceful end clears it; self-exit may too).
        st.with_store(|s| {
            s.remove_session_pubkeys_for_session(&sid).ok();
        });
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
    // Bounded: `resubscribe` opens a relay subscription, which can hang on a slow
    // or unreachable relay. `ensure_subscription` is awaited on hook-critical
    // paths (session_start, spawn_session), so a hang would block the editor.
    // The intent (the project is in `subscribed_projects`) is recorded above; on
    // timeout we fail open and the next session event re-runs resubscribe.
    match tokio::time::timeout(std::time::Duration::from_secs(5), resubscribe(state)).await {
        Ok(r) => r,
        Err(_) => {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] resubscribe timed out for {project} (best-effort)");
            }
            Ok(())
        }
    }
}

/// Rebuild and apply the union subscription across all hosted agents/projects.
async fn resubscribe(state: &Arc<DaemonState>) -> Result<()> {
    let hosted = state.hosted_pubkeys();
    let session_pks = state.live_session_pubkeys();

    // Authors for kind:0 / kind:30315 include both durable agent keys and active
    // transient session keys so peers receive session-signed presence.
    let mut authors: Vec<String> = hosted.clone();
    authors.extend(session_pks.clone());
    authors.sort_unstable();
    authors.dedup();

    let projects = state.subscribed_projects.lock().unwrap().clone();
    let owners = state.owners.clone();

    // All pubkeys that should receive p-tagged mentions: durable + session.
    let mut all_me: Vec<String> = hosted.clone();
    all_me.extend(session_pks);
    all_me.sort_unstable();
    all_me.dedup();

    for project in &projects {
        if all_me.is_empty() {
            let scope = crate::fabric::Scope {
                authors: authors.clone(),
                project: Some(project.clone()),
                mentions_to: None,
                owners: owners.clone(),
            };
            state.provider.subscribe(scope).await?;
        } else {
            for me in &all_me {
                let scope = crate::fabric::Scope {
                    authors: authors.clone(),
                    project: Some(project.clone()),
                    mentions_to: Some(me.clone()),
                    owners: owners.clone(),
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
            // Read the persisted session pubkey BEFORE deleting its row — it is
            // the authoritative value. Re-deriving from session_aliases is only a
            // fallback for rows written before this column existed; preferring the
            // stored pubkey avoids any chance of removing the wrong key (and thus
            // stranding the real one as a live member) if the recovered anchor
            // ever diverges from what session_start used.
            let stored_pubkey = state.with_store(|s| s.session_pubkey_for_session(&session_id));
            state.with_store(|s| {
                s.end_session(&session_id, now).ok();
                s.mark_session_dead(&session_id).ok();
                // Clear DB routing row for the dead session's transient pubkey.
                s.remove_session_pubkeys_for_session(&session_id).ok();
            });
            // Crash-GC: remove the session pubkey from the NIP-29 group.
            if let Some(nsec) = state.cfg.session_ikm_nsec().cloned() {
                if let Ok(op_keys) = nostr_sdk::prelude::Keys::parse(&nsec) {
                    let session_pubkey = stored_pubkey.unwrap_or_else(|| {
                        // Fallback: re-derive. Anchor recovered from session_aliases:
                        //   claude-code / codex → (harness, native_id)
                        //   opencode → anchor = session_id (resume alias only)
                        //   unknown / no rows → ("unknown", session_id)
                        let (harness_kind, anchor) =
                            state.with_store(|s| s.get_session_derivation_anchor(&session_id));
                        identity::derive_session_keys(
                            op_keys.secret_key(),
                            &snap.project,
                            &snap.agent_slug,
                            &harness_kind,
                            &anchor,
                        )
                        .public_key()
                        .to_hex()
                    });
                    let provider = state.provider.clone();
                    let store = state.store.clone();
                    let project = snap.project.clone();
                    tokio::spawn(async move {
                        provider
                            .nip29_remove_member(&project, &session_pubkey)
                            .await;
                        store
                            .lock()
                            .unwrap()
                            .remove_group_member(&project, &session_pubkey)
                            .ok();
                    });
                }
            }
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

        let (harness_kind, anchor) =
            state.with_store(|s| s.get_session_derivation_anchor(&session_id));
        let signer = match select_session_signer(
            state,
            &session_id,
            &id.pubkey_hex(),
            &snap.agent_slug,
            &snap.project,
            &harness_kind,
            &anchor,
        ) {
            Ok(signer) => signer,
            Err(e) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] signer selection failed while reconciling {}: {e:#}",
                        session_id
                    );
                }
                continue;
            }
        };
        if let Some(session_pubkey) = signer.transient_pubkey() {
            if let Err(e) = admit_transient_signer(state, &snap.project, session_pubkey).await {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] transient signer admission failed while reconciling {}: {e:#}",
                        session_id
                    );
                }
                state.release_session_signer(&session_id, &id.pubkey_hex(), &snap.project);
                state.with_store(|s| s.remove_session_pubkeys_for_session(&session_id).ok());
                continue;
            }
        }

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
            signer.session_keys(),
        );
        let _ = spawn_session(state, ep).await;
    }
    // Any registration/end transitions above enqueued publishes.
    state.status_outbox_notify.notify_waiters();
}

#[allow(clippy::too_many_arguments)]
fn engine_params_for(
    cfg: &Config,
    id: &AgentIdentity,
    agent_slug: &str,
    session_id: &str,
    project: &str,
    rel_cwd: &str,
    watch_pid: Option<i32>,
    // Derived keypair for a duplicate live session in the same routing scope.
    // `None` keeps the durable agent key as the default signer.
    session_keys: Option<Keys>,
) -> EngineParams {
    EngineParams {
        agent_slug: agent_slug.to_string(),
        agent_pubkey: id.pubkey_hex(),
        keys: id.keys.clone(),
        session_keys,
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
