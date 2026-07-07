use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;

const COLUMNS: &[(&str, &str)] = &[
    ("owner_backend_pubkey", "TEXT NOT NULL DEFAULT ''"),
    ("owner_host", "TEXT NOT NULL DEFAULT ''"),
];

pub(super) fn ensure_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(session_claims)")
        .context("reading session_claims columns")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;

    for (name, spec) in COLUMNS {
        if !existing.contains(*name) {
            conn.execute(
                &format!("ALTER TABLE session_claims ADD COLUMN {name} {spec}"),
                [],
            )
            .with_context(|| format!("adding session_claims.{name}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_owner_columns_to_existing_claims_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_claims (
                pubkey TEXT NOT NULL,
                base_pubkey TEXT NOT NULL,
                agent_slug TEXT NOT NULL DEFAULT '',
                ordinal INTEGER NOT NULL DEFAULT 0,
                session_id TEXT NOT NULL DEFAULT '',
                channel_h TEXT NOT NULL DEFAULT '',
                native_id TEXT NOT NULL DEFAULT '',
                harness TEXT NOT NULL DEFAULT '',
                last_active_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (pubkey, channel_h)
            )",
        )
        .unwrap();

        ensure_columns(&conn).unwrap();

        let cols = conn
            .prepare("PRAGMA table_info(session_claims)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<BTreeSet<_>>>()
            .unwrap();
        for (name, _) in COLUMNS {
            assert!(cols.contains(*name), "missing {name}");
        }
    }
}
