//! Schema 27: durable retirement work for reclaimed session handles.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    let tx = conn.transaction().context("starting schema-26 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS profile_retirements (
            pubkey TEXT PRIMARY KEY CHECK (pubkey <> ''),
            due_at INTEGER NOT NULL
        );
        PRAGMA user_version = 27;
        "#,
    )?;
    tx.commit().context("committing schema-26 migration")
}
