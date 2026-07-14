use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(super) const SCHEMA_VERSION: u32 = 5;

pub(super) fn stamp(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("stamping schema version")
}

pub(super) fn check(conn: &Connection, path: &Path) -> Result<()> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading schema user_version")?;
    let has_tables = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sqlite_master
                WHERE type='table' AND name NOT LIKE 'sqlite_%'
            )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .context("checking for existing schema tables")?;
    if version == 0 && has_tables {
        anyhow::bail!(
            "refusing to open {}: existing state.db has no schema version stamp; \
             move it aside or export non-rebuildable local state before rebuilding",
            path.display()
        );
    }
    if version != 0 && version != SCHEMA_VERSION {
        anyhow::bail!(
            "refusing to open {}: schema version {version} is incompatible with expected {SCHEMA_VERSION}",
            path.display()
        );
    }
    Ok(())
}
