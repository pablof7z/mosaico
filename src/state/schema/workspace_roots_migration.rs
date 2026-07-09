use anyhow::{Context, Result};
use rusqlite::Connection;

/// Rename the legacy `project_roots` table to `workspace_roots`, PRESERVING its
/// rows (the local channel -> on-disk workspace path bindings). `project_roots`
/// was local, non-rebuildable state, so this is a real rename, never a drop.
///
/// The DDL always `CREATE TABLE IF NOT EXISTS workspace_roots`, so on a fresh DB
/// (or one already migrated) `workspace_roots` exists and `project_roots` does
/// not — this is a no-op. When a legacy `project_roots` is present alongside the
/// freshly-created empty `workspace_roots`, its rows are copied over and the
/// legacy table dropped.
pub(super) fn ensure_renamed(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "project_roots")? {
        // Fresh or already-migrated DB: nothing legacy to carry over.
        return Ok(());
    }
    // The DDL created an (empty) `workspace_roots`; move the legacy rows into it,
    // then drop the legacy table so the wording is gone from the schema too.
    conn.execute_batch(
        r#"
        INSERT OR IGNORE INTO workspace_roots (channel_h, abs_path, updated_at)
            SELECT channel_h, abs_path, updated_at FROM project_roots;
        DROP TABLE project_roots;
        "#,
    )
    .context("migrating project_roots rows into workspace_roots")?;
    Ok(())
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )
        .with_context(|| format!("checking for {name} table"))?;
    Ok(exists)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_rows_and_drops_old_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE project_roots (
                channel_h TEXT PRIMARY KEY, abs_path TEXT NOT NULL, updated_at INTEGER NOT NULL
            );
            INSERT INTO project_roots VALUES ('h1', '/abs/one', 10);
            CREATE TABLE workspace_roots (
                channel_h TEXT PRIMARY KEY, abs_path TEXT NOT NULL, updated_at INTEGER NOT NULL
            );
            "#,
        )
        .unwrap();
        ensure_renamed(&conn).unwrap();
        assert!(!table_exists(&conn, "project_roots").unwrap());
        let path: String = conn
            .query_row(
                "SELECT abs_path FROM workspace_roots WHERE channel_h='h1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(path, "/abs/one");
    }

    #[test]
    fn fresh_db_is_noop() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE workspace_roots (channel_h TEXT PRIMARY KEY, abs_path TEXT NOT NULL, updated_at INTEGER NOT NULL);",
        )
        .unwrap();
        ensure_renamed(&conn).unwrap();
        assert!(table_exists(&conn, "workspace_roots").unwrap());
    }
}
