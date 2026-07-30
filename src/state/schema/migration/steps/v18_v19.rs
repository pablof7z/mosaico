use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    super::steps::require_shape(
        conn,
        18,
        "messages",
        &[
            "message_id",
            "thread_id",
            "channel_h",
            "author_pubkey",
            "body",
            "created_at",
            "direction",
            "sync_state",
            "native_event_id",
            "error",
        ],
        &["attachment_dir"],
    )?;
    let tx = conn.transaction().context("starting schema-18 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE messages
            ADD COLUMN attachment_dir TEXT NOT NULL DEFAULT '';
        PRAGMA user_version = 19;
        "#,
    )?;
    tx.commit().context("committing schema-18 migration")
}
