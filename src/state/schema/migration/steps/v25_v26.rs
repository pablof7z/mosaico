//! Schema 26: rename session_coaching to session_notice.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    if table_exists(conn, "session_notice")? {
        return stamp(conn);
    }
    super::steps::require_shape(
        conn,
        25,
        "session_coaching",
        &["pubkey", "runtime_generation", "code", "shown_at"],
        &[],
    )?;
    let tx = conn.transaction().context("starting schema-25 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE session_notice (
            pubkey             TEXT NOT NULL,
            runtime_generation INTEGER NOT NULL CHECK (runtime_generation > 0),
            code               TEXT NOT NULL CHECK (code <> ''),
            shown_at           INTEGER NOT NULL,
            PRIMARY KEY (pubkey, runtime_generation, code)
        );
        INSERT INTO session_notice SELECT * FROM session_coaching;
        DROP TABLE session_coaching;
        PRAGMA user_version = 26;
        "#,
    )?;
    tx.commit().context("committing schema-25 migration")
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .context("checking session_notice table existence")
}

fn stamp(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "user_version", 26)
        .context("stamping schema-26 migration")
}
