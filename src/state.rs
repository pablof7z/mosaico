//! Local persistence in SQLite (the persistence foundation).
//!
//! The store is two things and nothing else:
//!   1. `relay_*` materialized caches — channels, members, profiles, status, and
//!      a verbatim event log. Every one is rebuildable from the relay and is
//!      identical for local and remote agents. Agent identity, status, channels,
//!      and membership are NEVER authoritative local tables; they are caches.
//!   2. local plumbing the relay can't carry — OS process handles (`sessions`),
//!      joined-channel state (`session_channels`), external-id aliases
//!      (`session_aliases`), derived signing keys (`identities`), the inbound
//!      routing ledger (`inbox`), the outbound publish queue (`outbox`), and
//!      on-disk project paths (`project_roots`).
//!
//! A pubkey appears AT MOST ONCE per channel. Canonical session identity is
//! daemon-minted and stable; harness-native ids are aliases that repoint to the
//! newest live owner; every turn/session mutation resolves a raw external id to
//! the canonical id BEFORE writing. A session has one active publishing channel
//! (`sessions.channel_h`) and may listen in additional joined channels
//! (`session_channels`).

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Store {
    conn: Connection,
}

// ── relay_* cache row types ──────────────────────────────────────────────────

/// kind:39000 group metadata. A channel and a project are one abstraction;
/// `parent` is the only distinction (`""` = top-level project channel).
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
    /// A ROOT project (`parent` empty) uses its slug as BOTH its NIP-29 group id
    /// and its `name` (`channel_h == name`), so the slug IS the human label.
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

/// kind:0 metadata for any pubkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub pubkey: String,
    pub name: String,
    pub slug: String,
    pub host: String,
    pub is_backend: bool,
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

// ── local plumbing row types ─────────────────────────────────────────────────

/// A local agent process THIS daemon hosts. OS handles only — never agent
/// identity (that lives in `relay_status`/`relay_profiles`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub session_id: String,
    pub agent_pubkey: String,
    pub agent_slug: String,
    pub channel_h: String,
    pub harness: String,
    pub child_pid: Option<i32>,
    pub transcript_path: Option<String>,
    pub alive: bool,
    pub created_at: u64,
    pub last_seen: u64,
    pub working: bool,
    pub turn_started_at: u64,
    pub last_distill_at: u64,
    pub seen_cursor: u64,
    pub title: String,
    pub activity: String,
    pub resume_id: String,
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

/// A derived signing key the daemon publishes as. Ordinal 0 == the base agent
/// key. Binds an ordinal/per-session pubkey to its owning agent/session and the
/// harness-native id used to resume it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub pubkey: String,
    pub base_pubkey: String,
    pub agent_slug: String,
    pub ordinal: u32,
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
    pub target_session: String,
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
}

// ── canonical id minting ─────────────────────────────────────────────────────
// No uuid crate, so canonical session ids are built from nanosecond wall-clock
// time + an in-process monotonic counter: `te-<nanos_hex>-<counter_hex>`. The
// counter prevents collisions inside tight backfill loops.

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mint a fresh canonical session id (daemon-owned, opaque, stable across harness
/// id rotation).
pub(super) fn mint_session_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let seq = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("te-{nanos:x}-{seq:x}")
}

mod schema;

mod aliases;
mod channels;
mod core;
mod events;
mod identities;
mod inbox;
mod members;
mod outbox;
mod profiles;
mod project_roots;
mod retention;
pub use retention::{
    RetentionPruneReport, COMPLETED_LEDGER_RETENTION_SECS, RELAY_EVENT_RETENTION_SECS,
};
mod sessions;
mod status;

#[cfg(test)]
#[path = "state/tests.rs"]
mod tests;
