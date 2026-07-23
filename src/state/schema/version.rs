use anyhow::{bail, Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(super) const SCHEMA_VERSION: u32 = 15;

pub(super) fn read(conn: &Connection) -> Result<u32> {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .context("reading schema user_version")
}

pub(super) fn stamp(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .context("stamping schema version")
}

pub(super) fn check_initial(conn: &Connection, path: &Path, oldest_supported: u32) -> Result<u32> {
    let version = read(conn)?;
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
    if version > SCHEMA_VERSION {
        bail!(
            "refusing to open {}: schema version {version} is newer than this binary's version {SCHEMA_VERSION}",
            path.display()
        );
    }
    if version != 0 && version < oldest_supported {
        bail!(
            "refusing to open {}: schema version {version} predates automatic migrations (oldest supported {oldest_supported})",
            path.display()
        );
    }
    Ok(version)
}
