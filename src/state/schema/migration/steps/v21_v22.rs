//! Schema 21 -> 22: discard provenance-free relay projections.
//!
//! These tables are rebuildable caches. Keeping their rows while inventing a
//! source event, relay set, or observation generation would turn unknown
//! provenance into false provenance, so the migration drops them whole. The
//! current schema recreates empty projection tables and the NMP observations
//! repopulate them from exact row transitions.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    let tx = conn
        .transaction()
        .context("starting schema-21 projection reset")?;
    tx.execute_batch(
        r#"
        DROP TABLE IF EXISTS relay_channel_members;
        DROP TABLE IF EXISTS relay_channel_member_sets;
        DROP TABLE IF EXISTS relay_channels;
        DROP TABLE IF EXISTS relay_profiles;
        DROP TABLE IF EXISTS relay_status;
        DROP TABLE IF EXISTS relay_status_sets;
        DROP TABLE IF EXISTS relay_events;
        DROP TABLE IF EXISTS relay_reactions;
        PRAGMA user_version = 22;
        "#,
    )?;
    tx.commit().context("committing schema-21 projection reset")
}
