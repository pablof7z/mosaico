//! The stamped persistence schema.
//! NIP-29 group metadata and rosters are deliberately absent: NMP owns those
//! views. Remaining `relay_*` migrations are independent. Local tables are
//! non-rebuildable daemon state:
//! runtime bindings and locators, inbox, event claims, channel
//! reservations, and workspace roots.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

mod ddl;
mod migration;
mod validate;
mod version;

pub(crate) use migration::{load_pending_writes, replace_pending_writes};

pub(super) fn initialize_file(conn: &mut Connection, path: &Path) -> Result<()> {
    let initial_version = version::read(conn)?;
    migration::upgrade(conn, path)?;
    if initial_version == version::SCHEMA_VERSION && has_user_tables(conn)? {
        validate::canonical(conn, Some(path))?;
    }
    create_schema(conn)?;
    validate::canonical(conn, Some(path))?;
    version::stamp(conn)
}

pub(super) fn initialize_memory(conn: &Connection) -> Result<()> {
    create_schema(conn)?;
    validate::canonical(conn, None)?;
    version::stamp(conn)
}

fn create_schema(conn: &Connection) -> Result<()> {
    for part in ddl::SCHEMA_PARTS {
        conn.execute_batch(part).context("creating schema")?;
    }
    Ok(())
}

fn has_user_tables(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master
            WHERE type='table' AND name NOT LIKE 'sqlite_%'
        )",
        [],
        |row| row.get(0),
    )
    .context("checking for existing schema tables")
}

#[cfg(test)]
mod tests;
