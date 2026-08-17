//! Local persistence in SQLite (the persistence foundation).
//! The store contains only local plumbing the relay cannot carry: OS process
//! handles (`sessions`),
//!      joined-channel state (`session_channels`), typed runtime locators
//!      (`session_locators`), signer material, public handle leases, the inbound
//!      delivery ledger (`inbox`), backend replay guards (`event_claims`), the
//!      pending channel-name reservations,
//!      generation-scoped progressive coaching claims,
//! and on-disk workspace paths (`workspace_roots`). Relay-owned state is read
//! directly from retained NMP observations.
//!
//! A pubkey appears AT MOST ONCE per channel and is the durable agent identity.
//! The pubkey is the sole session identity. Harness-native ids and PTY endpoints
//! are typed locators that point to it. Runtime execution belongs to one
//! immutable workspace root; every channel scope is an explicit
//! `session_channels` membership.
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Arc;

mod profile;
pub use profile::Profile;
#[cfg(test)]
mod test_group_delivery;
#[cfg(test)]
pub(crate) use test_group_delivery::{TestGroup, TestGroupDelivery, TestRelayDelivery};

pub struct Store {
    conn: Connection,
    nmp_views: Arc<crate::nmp_views::NmpViews>,
    profile_feed: Arc<crate::nmp_host::ProfileFeed>,
}

impl Store {
    /// Bind both NMP-backed views in one call: the legacy `NmpViews` mirror and
    /// the retained profile feed that owns `Store::get_profile`. The two are
    /// always bound together from the same live [`NmpHost`](crate::nmp_host::NmpHost).
    pub(crate) fn bind_nmp_views_and_feed(
        &mut self,
        views: Arc<crate::nmp_views::NmpViews>,
        host: Arc<crate::nmp_host::NmpHost>,
    ) {
        self.nmp_views = views;
        self.profile_feed = Arc::new(crate::nmp_host::ProfileFeed::new(host));
    }

    /// The retained profile feed driving `Store::get_profile`. The coverage
    /// refresh calls `set_members` on this `Arc`.
    pub(crate) fn profile_feed(&self) -> Arc<crate::nmp_host::ProfileFeed> {
        Arc::clone(&self.profile_feed)
    }
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
}

/// kind:30315 current activity for one agent session in one channel, projected
/// from the exact NMP observation that currently owns the Row. NMP applies
/// replacement and NIP-40 removal before this value reaches Mosaico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub pubkey: String,
    pub channel_h: String,
    pub slug: String,
    pub title: String,
    pub activity: String,
    /// Immutable launch workspace advertised by the remote session.
    pub workspace: String,
    /// Launch branch when known; empty for non-git or older peers.
    pub branch: String,
    pub state: crate::session_state::SessionState,
    pub state_since: u64,
    pub last_seen: u64,
    pub updated_at: u64,
    pub expiration: u64,
}

/// A verbatim relay event projected from NMP's current delivered Rows.
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

/// Current kind:9 row projected from NMP. The author's pubkey is the sole
/// sender identity; Mosaico does not persist message history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub message_id: String,
    pub channel_h: String,
    pub author_pubkey: String,
    pub body: String,
    pub created_at: u64,
    pub attachment_dir: String,
}

/// One observed `p`-tag recipient on a kind:9 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageRecipient {
    pub message_id: String,
    pub recipient_pubkey: String,
}

/// One current NIP-25 reaction plus the target body projected from NMP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionRow {
    pub reaction_id: String,
    pub target_message_id: String,
    pub channel_h: String,
    pub reactor_pubkey: String,
    pub emoji: String,
    pub created_at: u64,
    /// The reacted-to message body from the same NMP view. Reactions whose
    /// target is absent are excluded from `ReactionRow` queries.
    pub target_body: String,
}

/// Fields reserved before starting one local runtime. A second active runtime
/// for the same pubkey is rejected by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterSession {
    pub pubkey: String,
    pub observed_harness: String,
    pub agent_slug: String,
    /// Channel membership requested by this launch. Empty means unscoped.
    pub launch_channel_h: String,
    /// Immutable top-level workspace root for this durable session identity.
    pub work_root: String,
    pub child_pid: Option<i32>,
    pub now: u64,
}

/// Immutable runtime facts admitted alongside a session generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedRuntimeFacts {
    pub observed_harness: String,
    pub claimed_harness: String,
    pub preset: String,
    pub transport: String,
    pub endpoint_provenance: String,
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
    /// Zero for durable recovery locators; otherwise the sole runtime
    /// generation allowed to act through this endpoint.
    pub runtime_generation: u64,
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
    /// Joined from the canonical message row by `event_id`.
    pub attachment_dir: String,
}

mod agent_usage;
mod arrival_cursors;
mod locators;
pub(crate) use locators::{
    LOCATOR_ACP, LOCATOR_APP_SERVER, LOCATOR_NATIVE_RESUME, LOCATOR_PID, LOCATOR_PI_RPC,
    LOCATOR_PTY,
};
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
mod mcp_actors;
mod members;
mod message_search;
mod session_signers;
pub(crate) use message_search::{
    MessageSearchHit, MessageSearchPosition, MessageSearchQuery, MESSAGE_SEARCH_DEFAULT_LIMIT,
    MESSAGE_SEARCH_MAX_LIMIT,
};
mod messages;
mod native_turn_attempts;
pub use native_turn_attempts::{
    FinishNativeTurnAttempt, NativeTurnAttempt, NativeTurnDeliveryKind, NativeTurnOutcome,
    NewNativeTurnAttempt,
};
mod profiles;
mod workspace_roots;
pub use workspace_roots::WorkspaceBinding;
mod reactions;
mod reader;
pub(crate) mod work_start;
pub(crate) use reader::StoreReader;
pub mod receipts;
mod retention;
pub use retention::{RetentionPruneReport, COMPLETED_LEDGER_RETENTION_SECS};
mod session_coaching;
mod session_context;
mod session_cursor;
mod session_lifecycle;
mod session_native;
mod session_recovery;
pub use session_lifecycle::HEADLESS_IDLE_TIMEOUT_SECS;
mod session_resume;
mod session_routes;
pub use session_routes::ConfirmedAdmissionCommit;
mod session_standing;
pub use session_standing::{SessionStanding, StandingState};
mod session_title;
mod session_ty;
pub use session_ty::{
    PresentationState, RecoveryState, RuntimeState, Session, StopReason, WorkState,
};
mod sessions;
mod status;
#[cfg(test)]
mod tests;
