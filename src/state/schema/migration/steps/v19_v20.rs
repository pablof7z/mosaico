//! Schema 20: `messages.direction` is deleted.
//!
//! The column recorded whether Mosaico itself wrote a message, and it existed
//! only because Mosaico seeded its own outbound chat into `messages` before any
//! relay had seen it. That local row said `'outbound'`; the relay's later
//! materialization of the same event said `'inbound'`; and `record_message`
//! carried a `CASE WHEN messages.direction='outbound'` latch whose whole job
//! was to keep the two from overwriting each other.
//!
//! The optimistic mirror is gone -- NMP injects the accepted write into the
//! subscription Mosaico already holds (#1182), so there is exactly one writer
//! again. And the column was never load-bearing anyway: its only reader,
//! `should_render_reply_nudge`, filters `author_pubkey=?1` in the same
//! predicate and always passes the reading agent's OWN pubkey, so
//! `direction='outbound'` restated `author_pubkey` and nothing else. A local
//! flag that duplicates a signed field is a second authority on one fact.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::path::Path;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    super::steps::require_shape(
        conn,
        19,
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
            "attachment_dir",
        ],
        &[],
    )?;
    let tx = conn.transaction().context("starting schema-19 migration")?;
    // The index named `direction` too, so it is rebuilt rather than dropped:
    // `(author_pubkey, sync_state, created_at)` is exactly the predicate the
    // reply-nudge lookups still run.
    tx.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_messages_author_pubkey;
        ALTER TABLE messages DROP COLUMN direction;
        CREATE INDEX IF NOT EXISTS idx_messages_author_pubkey
            ON messages(author_pubkey, sync_state, created_at);
        PRAGMA user_version = 20;
        "#,
    )?;
    tx.commit().context("committing schema-19 migration")
}
