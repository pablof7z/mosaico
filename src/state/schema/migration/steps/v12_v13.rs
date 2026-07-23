use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    let relay_status_columns = conn
        .prepare("PRAGMA table_info(relay_status)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let tx = conn.transaction().context("starting schema-12 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE sessions
            ADD COLUMN state_changed_at INTEGER NOT NULL DEFAULT 0;
        UPDATE sessions SET state_changed_at = CASE
            WHEN work_state='working' AND turn_started_at>0 THEN turn_started_at
            WHEN runtime_state='stopped' AND stopped_at>0 THEN stopped_at
            WHEN idle_since>0 THEN idle_since
            ELSE created_at
        END;
        "#,
    )?;
    if !relay_status_columns.is_empty()
        && !relay_status_columns
            .iter()
            .any(|column| column == "state_since")
    {
        tx.execute_batch(
            r#"
            ALTER TABLE relay_status
                ADD COLUMN state_since INTEGER NOT NULL DEFAULT 0;
            UPDATE relay_status SET state_since=updated_at;
            "#,
        )?;
    }
    tx.pragma_update(None, "user_version", 13)?;
    tx.commit().context("committing schema-12 migration")
}
