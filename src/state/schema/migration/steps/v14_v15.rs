use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    super::require_shape(
        conn,
        14,
        "sessions",
        &["pubkey", "work_state", "turn_started_at", "turn_count"],
        &["busy_seconds"],
    )?;
    let tx = conn.transaction().context("starting schema-14 migration")?;
    tx.execute_batch(
        "ALTER TABLE sessions
             ADD COLUMN busy_seconds INTEGER NOT NULL DEFAULT 0;",
    )?;
    tx.pragma_update(None, "user_version", 15)?;
    tx.commit().context("committing schema-14 migration")
}
