//! Local persistence in SQLite (the persistence foundation).
//! The store is two things and nothing else:
//!   1. `relay_*` materialized caches — channels, members, profiles, roster,
//!      status, and a verbatim event log. Every one is rebuildable from the
//!      relay and is identical for local and remote agents.
//!   2. local plumbing the relay can't carry — OS process handles (`sessions`),
//!      joined-channel state (`session_channels`), typed runtime locators
//!      (`session_locators`), signer material, public handle leases, the inbound
//!      delivery ledger (`inbox`), backend replay guards (`event_claims`), the
//!      pending channel-name reservations,
//!      and on-disk workspace paths (`workspace_roots`).
//!
//! A pubkey appears AT MOST ONCE per channel and is the durable agent identity.
//! The pubkey is the sole session identity. Harness-native ids and PTY endpoints
//! are typed locators that point to it. A runtime has one active publishing
//! channel (`sessions.channel_h`) and may listen in additional joined channels
//! (`session_channels`).
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

mod profile;
pub use profile::Profile;

pub struct Store {
    conn: Connection,
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
    pub channel_h: String,
    pub slug: String,
    pub title: String,
    pub activity: String,
    pub state: crate::session_state::SessionState,
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

/// Fields reserved before starting one local runtime. A second active runtime
/// for the same pubkey is rejected by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterSession {
    pub pubkey: String,
    pub harness: String,
    pub agent_slug: String,
    pub channel_h: String,
    pub child_pid: Option<i32>,
    pub transcript_path: Option<String>,
    pub now: u64,
}

/// Aggregate local launch activity for one canonical agent profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUsage {
    pub agent_slug: String,
    pub recent_uses: u64,
    pub last_used: u64,
}

/// A typed host-local locator pointing to the sole session identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLocator {
    pub harness: String,
    pub locator_kind: String,
    pub locator_value: String,
    pub pubkey: String,
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

mod agent_roster;
pub use agent_roster::{AgentAvailability, AgentRoster};
mod agent_usage;
mod locators;
pub(crate) use locators::{LOCATOR_ACP, LOCATOR_NATIVE_RESUME, LOCATOR_PID, LOCATOR_PTY};
mod channel_readiness_attempts;
pub use channel_readiness_attempts::{ChannelReadinessAttempt, NewChannelReadinessAttempt};
mod channels;
mod schema;
pub use channels::{archived_channel_about, is_archived_channel_about, CHANNEL_ABOUT_MAX_CHARS};
pub(crate) use schema::{load_pending_writes, replace_pending_writes};
mod core;
mod event_claims;
mod events;
mod handle_leases;
mod inbox;
mod members;
mod session_signers;
pub use members::ChannelMemberSet;
mod messages;
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
mod session_membership_cleanup;
mod session_native;
mod session_resume;
mod session_title;
mod session_ty;
pub use session_ty::Session;
mod sessions;
mod status;
#[cfg(test)]
mod tests;
mod turn_projection;
