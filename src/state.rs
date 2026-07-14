//! Local persistence in SQLite (the persistence foundation).
//! The store is two things and nothing else:
//!   1. `relay_*` materialized caches — channels, members, profiles, roster,
//!      status, and a verbatim event log. Every one is rebuildable from the
//!      relay and is identical for local and remote agents.
//!   2. local plumbing the relay can't carry — OS process handles (`sessions`),
//!      joined-channel state (`session_channels`), external-id aliases
//!      (`session_aliases`), signer material, session identities, the inbound
//!      delivery ledger (`inbox`), backend replay guards (`event_claims`), the
//!      outbound publish queue (`outbox`), pending channel-name reservations,
//!      and on-disk workspace paths (`workspace_roots`).
//!
//! A pubkey appears AT MOST ONCE per channel and is the durable agent identity.
//! Daemon-minted session ids and harness-native ids locate runtime incarnations;
//! they never own durable delivery state. A runtime has one active publishing
//! channel (`sessions.channel_h`) and may listen in additional joined channels
//! (`session_channels`).
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::cell::RefCell;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

mod profile;
pub use profile::Profile;

pub struct Store {
    conn: Connection,
    alias_writes: RefCell<aliases::provisional::AliasWriteState>,
}

/// kind:39000 group metadata. A channel is the one abstraction; `parent` is the
/// only distinction (`""` = a root channel at the top of the tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub channel_h: String,
    pub name: String,
    pub about: String,
    pub parent: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Channel {
    /// The channel's human display name, if it has one — the single source of
    /// truth for "is this channel named?".
    ///
    /// A ROOT channel (`parent` empty) keeps the workspace slug as its durable
    /// NIP-29 group id and uses `general` as its human channel name.
    /// A session/task room (`parent` set) whose `name` merely defaulted to its
    /// opaque id is genuinely unnamed. An empty `name` is always unnamed.
    pub fn human_name(&self) -> Option<&str> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        if !self.parent.is_empty() && name == self.channel_h {
            return None;
        }
        Some(name)
    }
}

/// kind:39001 (admins) / kind:39002 (members) row. `role` of `"admin"` is the
/// only management authority over the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelMember {
    pub channel_h: String,
    pub pubkey: String,
    pub role: String,
    pub updated_at: u64,
}

/// kind:30315 current activity for one agent session in one channel. A single
/// wire status may materialize to multiple rows when it carries multiple `h`
/// tags. Liveness is freshness: a row with `now > expiration` is NOT live
/// (NIP-40).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub pubkey: String,
    pub session_id: String,
    pub channel_h: String,
    pub slug: String,
    pub title: String,
    pub activity: String,
    pub busy: bool,
    pub last_seen: u64,
    pub updated_at: u64,
    pub expiration: u64,
}

/// A verbatim relay event (any kind other than 0 / 39xxx / 30315, which have
/// dedicated caches). NIP-01 replacement is applied on insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEvent {
    pub id: String,
    pub kind: u32,
    pub pubkey: String,
    pub created_at: u64,
    pub channel_h: String,
    pub d_tag: String,
    pub content: String,
    pub tags_json: String,
}

/// Canonical chat/message read-model row. The author's pubkey is the sole
/// durable sender identity; runtime incarnations never own message history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub message_id: String,
    pub thread_id: String,
    pub channel_h: String,
    pub author_pubkey: String,
    pub body: String,
    pub created_at: u64,
    pub direction: String,
    pub sync_state: String,
    pub native_event_id: Option<String>,
    pub error: Option<String>,
}

/// Input shape for recording a canonical message row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordMessage {
    pub message_id: String,
    pub thread_id: String,
    pub channel_h: String,
    pub author_pubkey: String,
    pub body: String,
    pub created_at: u64,
    pub direction: String,
    pub sync_state: String,
    pub native_event_id: Option<String>,
    pub error: Option<String>,
}

/// One recipient edge for a canonical message. The pubkey owns the durable
/// address; any runtime selected for immediate local delivery is ephemeral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecipient {
    pub message_id: String,
    pub recipient_pubkey: String,
    pub delivered_at: Option<u64>,
}

/// One materialized NIP-25 reaction (kind:7) plus the body of the message it
/// targets. Produced only by the materializer from a round-tripped relay event —
/// never optimistically fabricated. Surfaced as passive turn-start awareness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionRow {
    pub reaction_id: String,
    pub target_message_id: String,
    pub channel_h: String,
    pub reactor_pubkey: String,
    pub emoji: String,
    pub created_at: u64,
    /// The reacted-to message body (joined from `messages`). Empty when the
    /// target message is not (yet) in the local read model.
    pub target_body: String,
}

/// Fields for registering / reasserting a local session. The daemon resolves the
/// `(harness, external_id_kind, external_id)` alias to a canonical session;
/// missing aliases mint a fresh canonical id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterSession {
    pub harness: String,
    pub external_id_kind: String,
    pub external_id: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub channel_h: String,
    pub child_pid: Option<i32>,
    pub transcript_path: Option<String>,
    pub resume_id: String,
    pub now: u64,
}

/// An external id -> canonical session mapping (N:1, repointable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAlias {
    pub harness: String,
    pub external_id_kind: String,
    pub external_id: String,
    pub session_id: String,
    pub created_at: u64,
}

/// A session's public identity projection. Binds its authoritative pubkey to
/// local lifecycle and harness-native resume locators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub pubkey: String,
    pub agent_slug: String,
    /// Friendly code used as the leading component of the public handle.
    pub codename: String,
    pub session_id: String,
    pub channel_h: String,
    pub native_id: String,
    pub alive: bool,
    pub created_at: u64,
}

/// One inbound event addressed to a local agent, plus its delivery outcome. The
/// row's existence (and `state`) is the idempotency record — there is no separate
/// processed ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxRow {
    pub event_id: String,
    pub target_pubkey: String,
    pub state: String,
    pub from_pubkey: String,
    pub channel_h: String,
    pub body: String,
    pub created_at: u64,
    pub delivered_at: u64,
}

/// One queued outbound publish, retried until the relay acks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub local_id: i64,
    pub event_json: String,
    pub state: String,
    pub retries: i64,
    pub last_error: Option<String>,
    pub enqueued_at: u64,
    pub next_attempt_at: u64, // earliest wall-clock second this row may be (re)attempted; 0 = due now
}

// Canonical ids use wall-clock nanos plus a monotonic counter: `te-<nanos_hex>-<counter_hex>`.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a fresh canonical session id (daemon-owned, opaque, stable across harness
/// id rotation).
pub(crate) fn mint_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("te-{nanos:x}-{seq:x}")
}

mod agent_roster;
pub use agent_roster::{AgentAvailability, AgentRoster};
mod aliases;
mod channel_readiness_attempts;
pub use channel_readiness_attempts::{ChannelReadinessAttempt, NewChannelReadinessAttempt};
mod channels;
mod schema;
pub use channels::{archived_channel_about, is_archived_channel_about, CHANNEL_ABOUT_MAX_CHARS};
mod core;
mod durable_agent_sessions;
mod event_claims;
mod events;
mod handle_leases;
mod identities;
mod inbox;
pub mod llm_calls;
mod members;
mod session_signers;
pub use members::ChannelMemberSet;
mod messages;
mod outbox;
pub use outbox::outbox_retry_delay_secs;
mod profiles;
mod workspace_roots;
pub use workspace_roots::WorkspaceBinding;
mod quarantine;
pub use quarantine::QuarantinedEvent;
mod reactions;
mod reader;
pub(crate) use reader::StoreReader;
pub mod receipts;
mod retention;
pub use retention::{
    RetentionPruneReport, COMPLETED_LEDGER_RETENTION_SECS, RELAY_EVENT_RETENTION_SECS,
};
mod session_chat;
pub(crate) mod session_claims;
mod session_distill;
mod session_membership_cleanup;
mod session_native;
mod session_resume;
mod session_ty;
mod session_work_topic;
pub use session_ty::Session;
mod sessions;
mod status;
#[cfg(test)]
mod tests;
pub mod trellis_commits;
pub mod trellis_replay_capsules;
mod turn_projection;
