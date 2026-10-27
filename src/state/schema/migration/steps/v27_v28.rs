//! Schema 28: durable attempt count and next-retry time for event claims.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    if has_column(conn, "attempts")? && has_column(conn, "retry_at")? {
        return conn
            .pragma_update(None, "user_version", 28)
            .context("stamping schema-28 migration");
    }
    super::steps::require_shape(
        conn,
        27,
        "event_claims",
        &["event_id", "claim_key", "state", "updated_at"],
        &["attempts", "retry_at"],
    )?;
    let tx = conn.transaction().context("starting schema-28 migration")?;
    // Schemas 26 and 27 stamped a failed orchestration or session-dispatch
    // target with updated_at=0 (due at once) and an offline mention with the
    // failure time (due 30s later). retry_at=updated_at+30 keeps both, so each pending
    // row counts as one attempt and is due when it already was.
    tx.execute_batch(
        r#"
        ALTER TABLE event_claims ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE event_claims ADD COLUMN retry_at INTEGER NOT NULL DEFAULT 0;
        UPDATE event_claims SET attempts=1, retry_at=updated_at+30 WHERE state='pending';
        PRAGMA user_version = 28;
        "#,
    )?;
    tx.commit().context("committing schema-28 migration")
}

fn has_column(conn: &Connection, column: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('event_claims') WHERE name=?1)",
        [column],
        |row| row.get(0),
    )
    .context("checking event_claims retry columns")
}
