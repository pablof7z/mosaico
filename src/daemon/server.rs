//! The daemon process: sole owner of state.db AND the single relay connection.
//!
//! Started as the hidden daemon subcommand by a thin client's spawn-if-absent
//! path. See docs/daemon-design.md for responsibilities and lifecycle.

use super::client::StartupLock;
use super::protocol::{
    protocol_version, Hello, PleaseExit, Request, Response, Welcome, ERR_PROTOCOL_SKEW,
};
use super::tail_event::TailEvent;
use super::{socket_path, store_path};
use crate::config::{self, Config};
use crate::domain::{ChatMessage, DomainEvent};
use crate::fabric::provider::Nip29Provider;
use crate::identity::{self, AgentIdentity};
use crate::runtime::{self, EngineParams};
use crate::session::Harness;
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, pubkey_short};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys, RelayMessage, RelayPoolNotification};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

mod background;
mod demux;
mod invite_rpc;
mod management_command;
mod membership_cleanup;
mod orchestration_handler;
mod rpc;
mod session_signer;
mod tmux_rpc;

use background::{spawn_pruner, spawn_trellis_oracle_sampler};
use demux::spawn_demux;
use management_command::{handle_management_command, is_management_command_for_backend};
use orchestration_handler::handle_orchestration;

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

type StatusTailKey = (String, String, String);
type StatusTailSnapshot = (String, bool);

/// Shared daemon state. Store guards are held only across synchronous rusqlite
/// calls, never across `.await`. One process + one connection = one writer.
pub struct DaemonState {
    store: Arc<Mutex<Store>>,
    transport: Arc<Transport>,
    provider: Arc<Nip29Provider>,
    cfg: Config,
    host: String,
    owners: Vec<String>,
    hosted: Mutex<HashMap<String, HostedAgent>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    subscribed_projects: Mutex<Vec<String>>,
    subs: Mutex<crate::reconcile::SubscriptionReconciler>,
    status: Arc<Mutex<crate::reconcile::StatusReconciler>>,
    turn_lifecycle: Mutex<crate::reconcile::TurnLifecycleReconciler>,
    cursor: Mutex<crate::reconcile::CursorReconciler>,
    session_start: Mutex<crate::reconcile::SessionStartReconciler>,
    outbox: Arc<Mutex<crate::reconcile::OutboxReconciler>>,
    hook_contexts: crate::turn_context::HookContextGraphs,
    tail_tx: tokio::sync::broadcast::Sender<TailEvent>,
    open_clients: Mutex<u64>,
    shutdown: Notify,
    /// Peer presence join/leave tracking, keyed by `(pubkey, session_id, channel)`.
    /// A single session status can carry several `h` tags; each channel gets a
    /// tail-facing presence row.
    peer_sessions: Mutex<HashMap<(String, String, String), PeerTracked>>,
    /// Bounded first-sight tracking of native event ids: the relay pool
    /// notifies once per matching subscription, so the same event arrives many
    /// times. Set + insertion-order queue, capped at SEEN_EVENTS_CAP.
    seen_events: Mutex<(
        std::collections::HashSet<String>,
        std::collections::VecDeque<String>,
    )>,
    /// Pubkeys for which a Profile event has already been emitted, for first-seen dedup.
    seen_profiles: Mutex<std::collections::HashSet<String>>,
    /// Last-seen (title, active) keyed by `(author_pubkey, session_id, channel)`
    /// for tail dedup. Tracking `active` too means an active→idle flip emits a
    /// tail event even though the persistent title text is unchanged.
    last_status: Mutex<HashMap<StatusTailKey, StatusTailSnapshot>>,
    /// Wakes the status-outbox drainer the instant a transition enqueues a publish.
    outbox_notify: Notify,
    /// Per-session derived keypairs for duplicate live signers. The durable
    /// agent key remains the default; this map is populated only when a second
    /// live session of the same durable agent joins the same routing scope.
    session_keys: Mutex<HashMap<String, Keys>>,
    /// Reserved durable signer slots keyed by `(durable agent pubkey, group)`.
    /// Guards collision detection and reservation so simultaneous duplicate
    /// starts cannot both pick the durable signer.
    session_signers: Mutex<session_signer::SignerReservations>,
}

impl DaemonState {
    /// Hex pubkey of this backend's identity key. Ensures the daemon-owned
    /// management key exists before deriving the pubkey.
    fn backend_pubkey(&self) -> Option<String> {
        self.provider.management_pubkey()
    }

    /// Management signer for NIP-29 group operations. Goes through the provider
    /// so missing `tenexPrivateKey` is created in the same provisioning path.
    fn management_keys(&self) -> Result<Keys> {
        self.provider
            .management_keys()
            .ok_or_else(|| anyhow::anyhow!("no signing key (tenexPrivateKey) set"))
    }
    pub(crate) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
    /// The operator's whitelisted human pubkeys (config `whitelistedPubkeys`).
    /// Used to classify a mention's sender as a human vs another agent, which
    /// drives envelope presentation.
    pub(crate) fn whitelisted_pubkeys(&self) -> &[String] {
        &self.cfg.whitelisted_pubkeys
    }
    pub(crate) fn emit_delivery_failure(
        &self,
        project: &str,
        agent: &str,
        session: &str,
        detail: impl Into<String>,
    ) {
        self.emit_tail(TailEvent::delivery_failure(
            now_secs(),
            project,
            agent,
            session,
            detail,
        ));
    }
    pub(crate) fn fabric_provider(&self) -> &Nip29Provider {
        self.provider.as_ref()
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
    /// The authoritative agent-instance identity for a hosted session (issue #98).
    /// Prefers the bound `identities`-row projection; falls back to the base
    /// instance from the session row when no derived identity is bound yet. Every
    /// publisher/renderer/router consumes THIS instead of re-deriving label/pubkey
    /// policy from `agent_slug`/`agent_pubkey` + `keys_for_session(..)` fallbacks.
    pub(in crate::daemon) fn session_instance(
        &self,
        rec: &crate::state::Session,
    ) -> crate::identity::AgentInstance {
        self.with_store(|s| {
            s.instance_identity_for_session(&rec.session_id)
                .ok()
                .flatten()
        })
        .unwrap_or_else(|| {
            crate::identity::AgentInstance::base(rec.agent_slug.clone(), rec.agent_pubkey.clone())
        })
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
    /// Release a session's ordinal reservation + engine keys. Scans by session
    /// id (the ordinal slot is keyed by base pubkey + room + ordinal, all of
    /// which the reservation map already holds).
    fn release_session_signer(&self, session_id: &str) -> Option<Keys> {
        let mut reservations = self.session_signers.lock().unwrap();
        let mut session_keys = self.session_keys.lock().unwrap();
        session_signer::release(&mut reservations, &mut session_keys, session_id)
    }
}

// ── entry point ──────────────────────────────────────────────────────────────
mod channel_membership_rpc;
mod channel_resolve;
mod channels_rpc;
mod chat_read_tail;
mod chat_target;
mod chat_write;
mod cursor;
mod diagnostics;
mod engine_lifecycle;
mod lifecycle;
mod probe;
mod profile_rpc;
mod proposal;
mod resolution;
mod session_end;
mod session_signing;
mod session_start;
mod status_publish;
mod statusline;
mod subscriptions;
#[cfg(test)]
mod test_support;
mod turn_lifecycle;
mod turns;
mod who;

use channel_membership_rpc::{rpc_channels_join, rpc_channels_leave, rpc_channels_switch};
use channel_resolve::{
    project_root, resolve_channel, resolve_channel_ref, rpc_channels_resolve, ChannelResolution,
};
use channels_rpc::{
    ensure_session_room, rpc_channels_archive, rpc_channels_create, rpc_channels_edit,
    rpc_channels_list,
};
use chat_read_tail::{handle_chat_read, handle_tail};
use chat_write::rpc_chat_write;
use diagnostics::{
    log_nip29_role_decision, refresh_project_members_cache, rpc_debug_outbox, rpc_doctor,
    rpc_explain, rpc_local_backend,
};
use engine_lifecycle::{cancel_session, engine_params_for, reconcile_sessions, spawn_session};
pub use lifecycle::run;
use lifecycle::{write_json, ClientGuard, InitProgress};
use profile_rpc::{
    resolve_backend_pubkey, resolve_project_member_pubkey_hex, resolve_pubkey_hex,
    rpc_publish_profile,
};
use proposal::rpc_propose;
use resolution::{resolve_session, resolve_session_inner, CallerAnchor, ResolveScope};
use session_end::rpc_session_end;
use session_signing::{admit_ordinal_signer, select_session_signer};
use session_start::rpc_session_start;
use status_publish::spawn_outbox_drainer;
use statusline::rpc_statusline;
use subscriptions::{ensure_subscription, replay_channel_chat, resubscribe};
use turns::{rpc_turn_check, rpc_turn_end, rpc_turn_start};
use who::rpc_who;

async fn dispatch(state: &Arc<DaemonState>, req: &Request) -> Response {
    let result = match req.method.as_str() {
        "ping" => Ok(serde_json::json!({"pong": true})),
        "shutdown" => {
            state.shutdown.notify_waiters();
            Ok(serde_json::json!({"stopped": true}))
        }
        "who" => rpc_who(state, &req.params),
        "session_start" => rpc_session_start(state, &req.params, None).await,
        "session_end" => rpc_session_end(state, &req.params).await,
        "chat_write" => rpc_chat_write(state, &req.params).await,
        "publish" => rpc_propose(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params),
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "explain" => rpc_explain(state, &req.params),
        "probe" => probe::rpc_probe(state, &req.params),
        "local_backend" => rpc_local_backend(state),
        "project_list" => rpc::rpc_project_list(state).await,
        "project_edit" => rpc::rpc_project_edit(state, &req.params).await,
        "project_members" => rpc::rpc_project_members(state, &req.params).await,
        "project_add" => rpc::rpc_project_add(state, &req.params).await,
        "project_remove" => rpc::rpc_project_remove(state, &req.params).await,
        "agents_list_sessions" => rpc::rpc_agents_list_sessions(state, &req.params),
        "debug_outbox" => rpc_debug_outbox(state, &req.params),
        "channels_create" => rpc_channels_create(state, &req.params).await,
        "channels_edit" => rpc_channels_edit(state, &req.params).await,
        "channels_resolve" => rpc_channels_resolve(state, &req.params).await,
        "channels_list" => rpc_channels_list(state, &req.params),
        "channels_archive" => rpc_channels_archive(state, &req.params).await,
        "channels_join" => rpc_channels_join(state, &req.params).await,
        "channels_leave" => rpc_channels_leave(state, &req.params).await,
        "channels_switch" => rpc_channels_switch(state, &req.params).await,
        "publish_profile" => rpc_publish_profile(state, &req.params).await,
        "statusline" => rpc_statusline(state, &req.params),
        "tmux_status" => tmux_rpc::rpc_tmux_status(state).await,
        "tmux_send" => tmux_rpc::rpc_tmux_send(state, &req.params).await,
        "tmux_spawn" => tmux_rpc::rpc_tmux_spawn(state, &req.params).await,
        "invite" => invite_rpc::rpc_invite(state, &req.params).await,
        "tmux_attach" => tmux_rpc::rpc_tmux_attach(state, &req.params),
        "tmux_resume" => tmux_rpc::rpc_tmux_resume(state, &req.params).await,
        "tmux_resumable" => tmux_rpc::rpc_tmux_resumable(state).await,
        other => Err(anyhow::anyhow!("unknown method {other}")),
    };
    match result {
        Ok(v) => Response::ok(req.id, v),
        Err(e) => Response::err(req.id, "rpc_error", format!("{e:#}")),
    }
}

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

fn chat_rows_to_json(store: &Store, rows: &[InboxRow]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|r| {
            // Sender slug is no longer stored on the row; resolve it from the
            // profile cache (empty -> host falls back to the short pubkey).
            let from_slug = store
                .resolve_slug_for_pubkey(&r.from_pubkey)
                .ok()
                .flatten()
                .unwrap_or_default();
            serde_json::json!({
                "from_slug": from_slug,
                "project": r.channel_h,
                "from_session": "",
                "host": "",
                "subject": "",
                "created_at": r.created_at,
                "id": crate::idref::event_short_id(&r.event_id),
                "mention_event_id": r.event_id,
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

fn status_ttl_duration() -> Duration {
    Duration::from_secs(env_u64(
        "TENEX_EDGE_STATUS_TTL_S",
        crate::domain::STATUS_TTL_SECS,
    ))
}
