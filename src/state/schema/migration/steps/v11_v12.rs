use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    let tx = conn.transaction().context("starting schema-11 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS mcp_actor_aliases (
            actor_key  TEXT PRIMARY KEY,
            actor_kind TEXT NOT NULL CHECK (actor_kind IN ('openai', 'grok')),
            pubkey     TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL,
            last_seen  INTEGER NOT NULL
        );
        PRAGMA user_version = 12;
        "#,
    )?;
    tx.commit().context("committing schema-11 migration")
}
