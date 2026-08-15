//! The daemon process: sole owner of state.db, NMP acquisition, and provider I/O.
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
use crate::state::Store;
use crate::util::{now_secs, pubkey_short};
use anyhow::{Context, Result};
use nostr::{Event, Keys};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;
mod agent_config;
mod agent_discovery;
mod agent_usage;
mod backend_profile;
mod background;
mod delivery_drive;
mod demux;
mod direct_mentions;
mod group_records;
mod invite_rpc;
mod managed_lifecycle;
mod management_command;
mod membership_cleanup;
mod my_session;
mod operator_sessions;
mod orchestration_handler;
mod pi_tools;
pub(crate) mod presence;
mod pty_rpc;
mod rpc;
pub(crate) mod session_delivery;
mod session_dispatch;
mod session_dispatch_handler;
mod session_records;
mod state;
use background::spawn_pruner;
use demux::spawn_demux;
use management_command::{handle_management_command, is_management_command_for_backend};
use orchestration_handler::handle_orchestration;
use session_dispatch_handler::handle_session_dispatch;
use session_records::{HostedAgent, PeerTracked, SessionHandle, StatusTailKey, StatusTailSnapshot};
use state::{
    AgentConfigState, CatalogState, ConnectionState, DedupState, ReconcilerState,
    SessionRuntimeState, SubscriptionState,
};
/// Shared daemon state. Store guards span synchronous rusqlite calls, never `.await`.
pub struct DaemonState {
    store: Arc<Mutex<Store>>,
    provider: Arc<RwLock<Arc<Nip29Provider>>>,
    nmp: Arc<RwLock<Arc<crate::nmp_host::NmpHost>>>,
    cfg: RwLock<Config>,
    config_reload: Mutex<()>,
    /// Serializes lifecycle-owned NIP-29 standing transitions. Relay writes
    /// are asynchronous, so an expired removal finishes before a concurrent
    /// exact-session re-admission decides whether it must add again.
    standing_sync: tokio::sync::Mutex<()>,
    /// Makes remote caller correlation and first-session creation atomic across
    /// concurrent MCP requests for the same conversation.
    mcp_actor_sync: tokio::sync::Mutex<()>,
    agent_config: AgentConfigState,
    catalog: CatalogState,
    runtime: SessionRuntimeState,
    subscriptions: SubscriptionState,
    reconcilers: ReconcilerState,
    connections: ConnectionState,
    dedup: DedupState,
}
impl DaemonState {
    /// Hex pubkey of the daemon-owned management identity.
    fn backend_pubkey(&self) -> Option<String> {
        self.provider().management_pubkey()
    }
    /// Management signer for NIP-29 group ops; provisions `mosaicoPrivateKey`.
    fn management_keys(&self) -> Result<Keys> {
        self.provider()
            .management_keys()
            .ok_or_else(|| anyhow::anyhow!("no signing key (mosaicoPrivateKey) set"))
    }
    pub(crate) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let g = self.store.lock().expect("store mutex poisoned");
        f(&g)
    }
    pub(crate) fn mutate_agent_config<R>(&self, operation: impl FnOnce() -> R) -> R {
        self.agent_config.mutate(operation)
    }
    pub(crate) fn per_session_rooms(&self) -> bool {
        self.config().per_session_rooms
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
    pub(crate) fn fabric_provider(&self) -> Arc<Nip29Provider> {
        self.provider()
    }
    fn hosted_pubkeys(&self) -> Vec<String> {
        self.runtime
            .hosted
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
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
mod channel_init;
mod channel_membership_rpc;
mod channel_move;
mod channel_read_tail;
mod channel_resolve;
mod channel_search;
mod channel_send;
mod channel_wait;
mod channels_rpc;
mod chat_target;
mod config_reload;
mod config_state;
mod coordination_reminder;
mod cross_project_boundary;
mod cursor;
mod diagnostics;
mod engine_lifecycle;
mod lifecycle;
mod mcp_actor;
mod profile_rpc;
mod resolution;
mod session_end;
mod session_pty_wrap;
mod session_signing;
pub(crate) mod session_start;
mod session_termination;
mod subscriptions;
#[cfg(test)]
mod test_support;
mod turn_lifecycle;
pub(crate) mod turns;
mod who;
use backend_profile::{publish_backend_profile, rpc_backend_profile_refresh};
use channel_membership_rpc::{rpc_channel_join, rpc_channel_leave};
use channel_read_tail::{handle_channel_read, handle_tail};
use channel_resolve::{
    absolute, resolve_channel_path, root_channel, rpc_channel_resolve, ChannelResolution,
};
use channel_send::rpc_channel_send;
use channels_rpc::{
    ensure_session_room, rpc_channel_archive, rpc_channel_create, rpc_channel_delete,
    rpc_channel_edit, rpc_channel_list,
};
use diagnostics::{log_nip29_role_decision, rpc_explain, rpc_local_backend};
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
use subscriptions::{ensure_subscription, replay_channel_chat, sync_subscriptions};
use turns::{rpc_turn_check, rpc_turn_end, rpc_turn_start};
use who::rpc_who;
mod dispatch;
use dispatch::dispatch;
const SEEN_EVENTS_CAP: usize = 4096;
impl DaemonState {
    /// True exactly once per native event id (bounded memory). Subsequent
    /// sightings — NMP notifying for every matching observation —
    /// return false and must be ignored.
    fn first_sight(&self, event_id: &str) -> bool {
        let mut g = self.dedup.events.lock().unwrap();
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
        self.connections.tail_tx.subscribe()
    }
    fn emit_tail(&self, ev: TailEvent) {
        let _ = self.connections.tail_tx.send(ev);
    }
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
fn presence_lease_ttl() -> Duration {
    Duration::from_secs(env_u64(
        "MOSAICO_PRESENCE_LEASE_TTL_S",
        crate::domain::PRESENCE_LEASE_TTL_SECS,
    ))
}
