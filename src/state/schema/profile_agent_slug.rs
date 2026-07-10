//! Idempotent migration: add `relay_profiles.agent_slug` for the raw agent kind
//! carried by kind:0 profile tags. The existing `slug` column remains the
//! display/routing handle, which can be a per-session `agent/session` handle.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;

pub(super) fn ensure_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(relay_profiles)")
        .context("reading relay_profiles columns")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;

    if !existing.contains("agent_slug") {
        conn.execute(
            "ALTER TABLE relay_profiles ADD COLUMN agent_slug TEXT NOT NULL DEFAULT ''",
            [],
        )
        .context("adding relay_profiles.agent_slug")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_agent_slug_to_existing_relay_profiles() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE relay_profiles (
                pubkey      TEXT PRIMARY KEY,
                name        TEXT NOT NULL DEFAULT '',
                slug        TEXT NOT NULL DEFAULT '',
                host        TEXT NOT NULL DEFAULT '',
                is_backend  INTEGER NOT NULL DEFAULT 0,
                updated_at  INTEGER NOT NULL
            )",
        )
        .unwrap();

        ensure_column(&conn).unwrap();
        ensure_column(&conn).unwrap();

        let cols = conn
            .prepare("PRAGMA table_info(relay_profiles)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .unwrap();
        assert!(cols.contains("agent_slug"));
    }
}
