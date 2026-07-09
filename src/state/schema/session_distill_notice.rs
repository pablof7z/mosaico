//! Idempotent migration: add `sessions.distill_fail_streak` /
//! `sessions.distill_notice_at`, tracking consecutive background status-title
//! generation failures so a throttled heads-up can be injected into the
//! agent's turn context. Mirrors the `outbox_backoff` / `trellis_commits`
//! `ensure_columns` pattern so an existing on-disk `sessions` table gains the
//! columns without a schema-version bump.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;

const COLUMNS: &[(&str, &str)] = &[
    ("distill_fail_streak", "INTEGER NOT NULL DEFAULT 0"),
    ("distill_notice_at", "INTEGER NOT NULL DEFAULT 0"),
];

pub(super) fn ensure_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(sessions)")
        .context("reading sessions columns")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;

    for (name, spec) in COLUMNS {
        if !existing.contains(*name) {
            conn.execute(
                &format!("ALTER TABLE sessions ADD COLUMN {name} {spec}"),
                [],
            )
            .with_context(|| format!("adding sessions.{name}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_distill_notice_columns_to_pre_migration_sessions() {
        let conn = Connection::open_in_memory().unwrap();
        // a sessions table shaped like the pre-migration schema (no distill
        // notice columns)
        conn.execute_batch(
            "CREATE TABLE sessions (
                session_id   TEXT PRIMARY KEY,
                agent_pubkey TEXT NOT NULL,
                created_at   INTEGER NOT NULL
            )",
        )
        .unwrap();

        ensure_columns(&conn).unwrap();
        // idempotent second run must not error
        ensure_columns(&conn).unwrap();

        let cols = conn
            .prepare("PRAGMA table_info(sessions)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .unwrap();
        assert!(cols.contains("distill_fail_streak"), "column not added");
        assert!(cols.contains("distill_notice_at"), "column not added");
    }
}
