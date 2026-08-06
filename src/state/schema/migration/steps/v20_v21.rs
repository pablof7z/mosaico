//! Schema 21: durable, generation-scoped progressive coaching claims.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    super::steps::require_shape(conn, 20, "sessions", &["pubkey", "runtime_generation"], &[])?;
    let tx = conn.transaction().context("starting schema-20 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE session_coaching (
            pubkey             TEXT NOT NULL,
            runtime_generation INTEGER NOT NULL CHECK (runtime_generation > 0),
            code               TEXT NOT NULL CHECK (code <> ''),
            shown_at           INTEGER NOT NULL,
            PRIMARY KEY (pubkey, runtime_generation, code)
        );
        PRAGMA user_version = 21;
        "#,
    )?;
    tx.commit().context("committing schema-20 migration")
}
