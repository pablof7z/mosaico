//! Schema 24: replace transport-owning launch bundles with argument presets.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    super::steps::require_shape(
        conn,
        23,
        "sessions",
        &["admitted_bundle", "admitted_transport"],
        &["admitted_preset"],
    )?;
    let tx = conn.transaction().context("starting schema-24 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE sessions DROP COLUMN admitted_bundle;
        ALTER TABLE sessions ADD COLUMN admitted_preset TEXT NOT NULL DEFAULT '';
        PRAGMA user_version = 24;
        "#,
    )?;
    tx.commit().context("committing schema-24 migration")
}
