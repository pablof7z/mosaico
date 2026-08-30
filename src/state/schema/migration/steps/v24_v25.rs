//! Schema 25: give the product-owned arrival ledger its canonical table name.

use std::path::Path;

use anyhow::{bail, Context, Result};
use rusqlite::Connection;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    if table_exists(conn, "event_arrivals")? {
        return stamp(conn);
    }

    let source = arrival_ledger(conn)?;
    let tx = conn.transaction().context("starting schema-25 migration")?;
    tx.execute_batch(&format!(
        "ALTER TABLE {} RENAME TO event_arrivals; PRAGMA user_version = 25;",
        quote_identifier(&source),
    ))?;
    tx.commit().context("committing schema-25 migration")
}

fn arrival_ledger(conn: &Connection) -> Result<String> {
    let mut statement = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let candidates = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|table| matches_arrival_ledger(conn, &table).map(|matches| matches.then_some(table)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();

    match candidates.as_slice() {
        [table] => Ok(table.clone()),
        [] => bail!("schema-24 database has no durable arrival ledger"),
        _ => bail!("schema-24 database has ambiguous durable arrival ledgers"),
    }
}

fn matches_arrival_ledger(conn: &Connection, table: &str) -> Result<bool> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({})", quote_identifier(table)))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(columns == ["sequence", "event_id"])
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .context("checking durable arrival ledger")
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn stamp(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "user_version", 25)
        .context("stamping schema-25 migration")
}
