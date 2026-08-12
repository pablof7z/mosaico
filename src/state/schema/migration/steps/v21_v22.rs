//! Schema 22: all relay-derived state leaves Mosaico persistence.
//!
//! NMP owns current groups, profiles, statuses, events, messages, recipients,
//! reactions, replacement selection, source evidence, and removal. This
//! migration retains only host-local arrival order and downloaded paths.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    let tx = conn.transaction().context("starting schema-22 migration")?;
    tx.execute_batch(
        r#"
        CREATE TABLE nmp_event_arrivals (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE
        );
        INSERT INTO nmp_event_arrivals (sequence, event_id)
        SELECT rowid, id FROM relay_events ORDER BY rowid;

        CREATE TABLE message_attachments (
            event_id  TEXT PRIMARY KEY,
            directory TEXT NOT NULL CHECK (directory <> '')
        );
        INSERT INTO message_attachments (event_id, directory)
        SELECT message_id, attachment_dir
          FROM messages
         WHERE attachment_dir <> '';

        DROP TABLE IF EXISTS relay_channel_member_sets;
        DROP TABLE IF EXISTS relay_channel_members;
        DROP TABLE IF EXISTS relay_channels;
        DROP TABLE IF EXISTS relay_profiles;
        DROP TABLE IF EXISTS relay_status_sets;
        DROP TABLE IF EXISTS relay_status;
        DROP TABLE IF EXISTS relay_reactions;
        DROP TABLE IF EXISTS message_recipients;
        DROP TABLE IF EXISTS messages;
        DROP TABLE IF EXISTS relay_events;
        PRAGMA user_version = 22;
        "#,
    )?;
    tx.commit().context("committing schema-22 migration")
}
