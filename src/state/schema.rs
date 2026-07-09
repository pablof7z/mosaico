//! The stamped persistence schema.
//! Six `relay_*` tables are materialized caches and may be dropped/rebuilt from
//! relay state. The remaining local tables are non-rebuildable daemon state:
//! session bindings, aliases, identities, inbox/outbox, channel reservations,
//! and workspace roots.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

mod ddl;
mod identity_migration;
mod outbox_backoff;
mod profile_agent_slug;
mod session_claims;
mod session_distill_notice;
mod trellis_commits;
mod trellis_replay_capsules;
mod version;
mod workspace_roots_migration;

use ddl::SCHEMA;

pub(super) fn initialize_file(conn: &Connection, path: &Path) -> Result<()> {
    version::check(conn, path)?;
    conn.execute_batch(SCHEMA).context("creating schema")?;
    identity_migration::ensure_session_primary_key(conn)?;
    session_claims::ensure_columns(conn)?;
    profile_agent_slug::ensure_column(conn)?;
    trellis_commits::ensure_columns(conn)?;
    outbox_backoff::ensure_columns(conn)?;
    session_distill_notice::ensure_columns(conn)?;
    trellis_replay_capsules::ensure_table(conn)?;
    workspace_roots_migration::ensure_renamed(conn)?;
    version::stamp(conn)
}

pub(super) fn initialize_memory(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)
        .context("creating in-memory schema")?;
    identity_migration::ensure_session_primary_key(conn)?;
    session_claims::ensure_columns(conn)?;
    profile_agent_slug::ensure_column(conn)?;
    trellis_commits::ensure_columns(conn)?;
    outbox_backoff::ensure_columns(conn)?;
    session_distill_notice::ensure_columns(conn)?;
    trellis_replay_capsules::ensure_table(conn)?;
    workspace_roots_migration::ensure_renamed(conn)?;
    version::stamp(conn)
}

#[cfg(test)]
mod tests;
