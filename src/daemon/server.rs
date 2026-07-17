//! The daemon process: sole owner of state.db, NMP acquisition, and provider I/O.
//!
//! Started as the hidden daemon subcommand by a thin client's spawn-if-absent
use super::client::StartupLock;
use super::protocol::{
    protocol_version, Hello, PleaseExit, Request, Response, Welcome, ERR_PROTOCOL_SKEW,
};
use super::tail_event::TailEvent;
use super::{socket_path, store_path};
use crate::config::{self, Config};
use crate::domain::{ChatMessage, DomainEvent};
use crate::fabric::provider::Nip29Provider;
use crate::identity;
use crate::runtime::{self, EngineParams};
use crate::session::Harness;
use crate::state::{InboxRow, Store};
use crate::transport::Transport;
use crate::util::{now_secs, pubkey_short};
use anyhow::{Context, Result};
use nostr_sdk::prelude::{Event, Keys};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
mod agent_discovery;
mod agent_roster;
mod agent_usage;
pub(crate) mod auto_reply;
mod background;
mod delivery_drive;
mod demux;
mod invite_rpc;
mod management_command;
mod membership_cleanup;
mod my_session;
mod operator_sessions;
mod orchestration_handler;
mod pty_rpc;
mod rpc;
mod session_dispatch;
mod session_dispatch_handler;
mod session_records;
use background::spawn_pruner;
use demux::{spawn_demux, warm_profiles};
use management_command::{handle_management_command, is_management_command_for_backend};
use orchestration_handler::handle_orchestration;
use session_dispatch_handler::handle_session_dispatch;
use session_records::{HostedAgent, PeerTracked, SessionHandle, StatusTailKey, StatusTailSnapshot};
/// Shared daemon state. Store guards span synchronous rusqlite calls, never `.await`.
pub struct DaemonState {
    store: Arc<Mutex<Store>>,
    transport: Arc<Transport>,
    provider: Arc<Nip29Provider>,
    nmp: Arc<crate::nmp_host::NmpHost>,
    cfg: Config,
    host: String,
    owners: Vec<String>,
    agent_catalog: Mutex<crate::agent_catalog::AgentCatalog>,
    installed_harnesses: Mutex<Vec<Harness>>,
    hosted: Mutex<HashMap<String, HostedAgent>>,
    sessions: Mutex<HashMap<String, SessionHandle>>,
    subscribed_root_channels: Mutex<Vec<String>>,
    subs: Mutex<crate::reconcile::SubscriptionReconciler>,
    subscription_sync: tokio::sync::Mutex<()>,
    status: Arc<Mutex<crate::reconcile::StatusReconciler>>,
    hook_contexts: crate::turn_context::HookContextStates,
    tail_tx: tokio::sync::broadcast::Sender<TailEvent>,
    open_clients: Mutex<u64>,
    shutdown: Notify,
    /// Peer presence join/leave tracking, keyed by `(pubkey, channel)`.
    /// A single identity status can carry several `h` tags; each channel gets a
    /// tail-facing presence row.
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
    /// Pubkeys with a kind:0 fetch in flight, so duplicate relay deliveries of the
    /// same event collapse to ONE warm. Entries clear when the fetch completes, so
    /// a failed (offline) fetch is retried on the next sighting.
    warming: Mutex<std::collections::HashSet<String>>,
    /// Last-seen (title, active) keyed by `(author_pubkey, channel)`
    /// for tail dedup. Tracking `active` too means an active→idle flip emits a
    /// tail event even though the persistent title text is unchanged.
    last_status: Mutex<HashMap<StatusTailKey, StatusTailSnapshot>>,
}
impl DaemonState {
    /// Hex pubkey of this backend's identity key. Ensures the daemon-owned
    /// management key exists before deriving the pubkey.
    fn backend_pubkey(&self) -> Option<String> {
        self.provider.management_pubkey()
    }

    /// Management signer for NIP-29 group ops; provisions `mosaicoPrivateKey`.
    fn management_keys(&self) -> Result<Keys> {
        self.provider
            .management_keys()
            .ok_or_else(|| anyhow::anyhow!("no signing key (mosaicoPrivateKey) set"))
    }
    pub(crate) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
    /// The operator's whitelisted human pubkeys (config `whitelistedPubkeys`);
    /// classify a mention's sender as human vs agent for envelope presentation.
    pub(crate) fn whitelisted_pubkeys(&self) -> &[String] {
        &self.cfg.whitelisted_pubkeys
    }
    pub(crate) fn per_session_rooms(&self) -> bool {
        self.cfg.per_session_rooms
    }
    pub(crate) fn emit_delivery_failure(
        &self,
        channel: &str,
        agent: &str,
        session: &str,
        detail: impl Into<String>,
    ) {
        self.emit_tail(TailEvent::delivery_failure(
            now_secs(),
            channel,
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
    /// The pubkey-owned read-side identity for a hosted session.
    pub(in crate::daemon) fn session_instance(
        &self,
        rec: &crate::state::Session,
    ) -> crate::identity::SessionIdentity {
        self.with_store(|store| store.session_identity(&rec.pubkey))
            .expect("session identity lookup failed")
            .expect("live session is missing its identity projection")
    }
}

// ── entry point ──────────────────────────────────────────────────────────────
mod channel_membership_rpc;
mod channel_read_tail;
mod channel_resolve;
mod channel_send;
mod channel_wait;
mod channels_rpc;
mod chat_target;
mod cursor;
mod diagnostics;
mod engine_lifecycle;
mod lifecycle;
mod profile_rpc;
mod resolution;
mod session_end;
mod session_pty_wrap;
mod session_signing;
pub(crate) mod session_start;
mod statusline;
mod subscriptions;
#[cfg(test)]
mod test_support;
mod turn_lifecycle;
mod turns;
mod who;
use agent_roster::{publish_local_agent_roster, rpc_agent_roster_refresh};
use channel_membership_rpc::{rpc_channel_join, rpc_channel_leave, rpc_channel_switch};
use channel_read_tail::{handle_channel_read, handle_tail};
use channel_resolve::{
    resolve_channel_path, resolve_channel_ref, root_channel, rpc_channel_resolve, ChannelResolution,
};
use channel_send::rpc_channel_send;
use channels_rpc::{
    ensure_session_room, rpc_channel_archive, rpc_channel_create, rpc_channel_edit,
    rpc_channel_list,
};
use diagnostics::{
    log_nip29_role_decision, refresh_channel_members_cache, rpc_doctor, rpc_explain,
    rpc_local_backend,
};
use engine_lifecycle::{cancel_session, engine_params_for, reconcile_sessions, spawn_session};
pub use lifecycle::run;
use lifecycle::{write_json, ClientGuard, InitProgress};
use my_session::{rpc_my_session, rpc_my_session_status};
use profile_rpc::{resolve_backend_pubkey, resolve_channel_member_pubkey_hex, resolve_pubkey_hex};
use resolution::{resolve_session, resolve_session_inner, CallerAnchor, ResolveScope};
use session_end::{rpc_session_end, rpc_session_kill};
use session_pty_wrap::rpc_session_pty_wrap;
use session_signing::retire_reclaimed_profile;
pub(crate) use session_signing::{
    load_session_identity, prepare_session_identity, PreparedIdentity,
};
use session_start::rpc_session_start;
use statusline::rpc_statusline;
use subscriptions::{ensure_subscription, replay_channel_chat, sync_subscriptions};
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
        "my_session" => rpc_my_session(state, &req.params),
        "my_session_status" => rpc_my_session_status(state, &req.params).await,
        "session_start" => rpc_session_start(state, &req.params, None).await,
        "session_end" => rpc_session_end(state, &req.params).await,
        "session_kill" => rpc_session_kill(state, &req.params).await,
        "session_pty_wrap" => rpc_session_pty_wrap(state, &req.params).await,
        "channel_send" => rpc_channel_send(state, &req.params).await,
        "channel_wait" => channel_wait::rpc_channel_wait(state, &req.params).await,
        "channel_reply" => channel_send::rpc_channel_reply(state, &req.params).await,
        "channel_react" => channel_send::rpc_channel_react(state, &req.params).await,
        "turn_start" => rpc_turn_start(state, &req.params).await,
        "turn_check" => rpc_turn_check(state, &req.params).await,
        "turn_end" => rpc_turn_end(state, &req.params).await,
        "doctor" => rpc_doctor(state).await,
        "explain" => rpc_explain(state, &req.params),
        "local_backend" => rpc_local_backend(state),
        "root_channels" => rpc::rpc_root_channels(state).await,
        "channel_members" => rpc::rpc_channel_members(state, &req.params).await,
        "channel_add_member" => rpc::rpc_channel_add_member(state, &req.params).await,
        "channel_remove_member" => rpc::rpc_channel_remove_member(state, &req.params).await,
        "operator_sessions" => operator_sessions::rpc_operator_sessions(state),
        "agent_usage" => agent_usage::rpc_agent_usage(state, &req.params),
        "pty_supervisor_exit" => rpc::rpc_pty_supervisor_exit(state, &req.params).await,
        "agent_roster_refresh" => rpc_agent_roster_refresh(state, &req.params),
        "channel_create" => rpc_channel_create(state, &req.params).await,
        "channel_edit" => rpc_channel_edit(state, &req.params).await,
        "channel_resolve" => rpc_channel_resolve(state, &req.params).await,
        "channel_list" => rpc_channel_list(state, &req.params),
        "channel_archive" => rpc_channel_archive(state, &req.params).await,
        "channel_join" => rpc_channel_join(state, &req.params).await,
        "channel_leave" => rpc_channel_leave(state, &req.params).await,
        "channel_switch" => rpc_channel_switch(state, &req.params).await,
        "dispatch" => session_dispatch::rpc_dispatch(state, &req.params).await,
        "statusline" => rpc_statusline(state, &req.params),
        "pty_status" => pty_rpc::rpc_pty_status(state).await,
        "pty_send" => pty_rpc::rpc_pty_send(state, &req.params).await,
        "pty_spawn" => pty_rpc::rpc_pty_spawn(state, &req.params).await,
        "pty_launch_existing" => pty_rpc::rpc_pty_launch_existing(state, &req.params).await,
        "invite" => invite_rpc::rpc_invite(state, &req.params).await,
        "pty_attach" => pty_rpc::rpc_pty_attach(state, &req.params),
        "pty_resume" => pty_rpc::rpc_pty_resume(state, &req.params).await,
        "pty_resumable" => pty_rpc::rpc_pty_resumable(state).await,
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
                "channel": r.channel_h,
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
        "MOSAICO_STATUS_TTL_S",
        crate::domain::STATUS_TTL_SECS,
    ))
}
