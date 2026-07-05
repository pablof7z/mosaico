use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn ensure_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS trellis_replay_capsules (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            surface        TEXT NOT NULL,
            trigger_kind   TEXT NOT NULL,
            trigger_ref    TEXT NOT NULL DEFAULT '',
            script_json    TEXT NOT NULL,
            script_bytes   INTEGER NOT NULL,
            format_version INTEGER NOT NULL DEFAULT 1,
            created_at     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_trellis_replay_capsules_surface
            ON trellis_replay_capsules(surface, created_at);",
    )
    .context("creating trellis_replay_capsules")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_missing_replay_capsule_table() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_table(&conn).unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type='table' AND name='trellis_replay_capsules'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists);
    }
}
