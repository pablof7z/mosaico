use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;

const COLUMNS: &[(&str, &str)] = &[
    ("mode", "TEXT NOT NULL DEFAULT ''"),
    ("trigger_ref", "TEXT NOT NULL DEFAULT ''"),
    ("resource_commands_json", "TEXT NOT NULL DEFAULT '[]'"),
    ("output_frames_json", "TEXT NOT NULL DEFAULT '[]'"),
    ("effect_count", "INTEGER NOT NULL DEFAULT 0"),
    ("suppressed_count", "INTEGER NOT NULL DEFAULT 0"),
    ("oracle_status", "TEXT"),
    ("oracle_error", "TEXT"),
    ("graph_resources", "INTEGER NOT NULL DEFAULT 0"),
];

pub(super) fn ensure_columns(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(trellis_commits)")
        .context("reading trellis_commits columns")?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;

    for (name, spec) in COLUMNS {
        if !existing.contains(*name) {
            conn.execute(
                &format!("ALTER TABLE trellis_commits ADD COLUMN {name} {spec}"),
                [],
            )
            .with_context(|| format!("adding trellis_commits.{name}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_missing_columns_to_existing_v1_ledger() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE trellis_commits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                surface TEXT NOT NULL,
                transaction_id INTEGER NOT NULL,
                revision INTEGER NOT NULL,
                trigger_kind TEXT NOT NULL,
                changed_inputs_json TEXT NOT NULL DEFAULT '[]',
                changed_derived_json TEXT NOT NULL DEFAULT '[]',
                changed_collections_json TEXT NOT NULL DEFAULT '[]',
                command_count INTEGER NOT NULL DEFAULT 0,
                output_count INTEGER NOT NULL DEFAULT 0,
                noop INTEGER NOT NULL DEFAULT 0,
                duration_us INTEGER NOT NULL DEFAULT 0,
                graph_nodes INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            )",
        )
        .unwrap();

        ensure_columns(&conn).unwrap();

        let cols = conn
            .prepare("PRAGMA table_info(trellis_commits)")
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
