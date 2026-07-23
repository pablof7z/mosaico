use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::journal;

mod v10_v11;
mod v11_v12;
mod v12_v13;
mod v5_v6;
mod v9_v10;

pub(super) fn v5_to_v6(conn: &mut Connection, path: &Path) -> Result<()> {
    v5_v6::migrate(conn, path)
}

pub(super) fn v9_to_v10(conn: &mut Connection, path: &Path) -> Result<()> {
    v9_v10::migrate(conn, path)
}

pub(super) fn v10_to_v11(conn: &mut Connection, path: &Path) -> Result<()> {
    v10_v11::migrate(conn, path)
}

pub(super) fn v11_to_v12(conn: &mut Connection, path: &Path) -> Result<()> {
    v11_v12::migrate(conn, path)
}

pub(super) fn v12_to_v13(conn: &mut Connection, path: &Path) -> Result<()> {
    v12_v13::migrate(conn, path)
}

pub(super) fn v4_to_v5(conn: &mut Connection, _path: &Path) -> Result<()> {
    require_shape(
        conn,
        4,
        "sessions",
        &["session_id", "agent_pubkey"],
        &["pubkey"],
    )?;
    require_shape(
        conn,
        4,
        "session_aliases",
        &["external_id", "session_id"],
        &[],
    )?;
    require_shape(
        conn,
        4,
        "messages",
        &["author_pubkey", "author_session"],
        &[],
    )?;
    require_shape(
        conn,
        4,
        "message_recipients",
        &["recipient_pubkey", "target_session"],
        &[],
    )?;
    let tx = conn.transaction().context("starting schema-4 migration")?;
    tx.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_messages_channel;
        DROP INDEX IF EXISTS idx_messages_native;
        DROP INDEX IF EXISTS idx_messages_author_session;
        DROP INDEX IF EXISTS idx_message_recipients_target;
        ALTER TABLE message_recipients RENAME TO migration_v4_message_recipients;
        ALTER TABLE messages RENAME TO migration_v4_messages;

        CREATE TABLE messages (
            message_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL, author_pubkey TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL,
            direction TEXT NOT NULL DEFAULT 'inbound',
            sync_state TEXT NOT NULL DEFAULT 'accepted', native_event_id TEXT,
            error TEXT
        );
        CREATE INDEX idx_messages_channel
            ON messages(channel_h, created_at, message_id);
        CREATE INDEX idx_messages_native ON messages(native_event_id);
        CREATE INDEX idx_messages_author_pubkey
            ON messages(author_pubkey, direction, sync_state, created_at);
        INSERT INTO messages
        SELECT message_id, thread_id, channel_h, author_pubkey, body, created_at,
               direction, sync_state, native_event_id, error
          FROM migration_v4_messages;

        CREATE TABLE message_recipients (
            message_id TEXT NOT NULL, recipient_pubkey TEXT NOT NULL,
            delivered_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (message_id, recipient_pubkey)
        );
        CREATE INDEX idx_message_recipients_pubkey
            ON message_recipients(recipient_pubkey, delivered_at);
        INSERT INTO message_recipients
        SELECT message_id, recipient_pubkey, MAX(delivered_at)
          FROM migration_v4_message_recipients
         GROUP BY message_id, recipient_pubkey;

        DROP TABLE migration_v4_messages;
        DROP TABLE migration_v4_message_recipients;
        PRAGMA user_version = 5;
        "#,
    )?;
    tx.commit().context("committing schema-4 migration")
}

pub(super) fn v6_to_v7(conn: &mut Connection, _path: &Path) -> Result<()> {
    require_shape(
        conn,
        6,
        "sessions",
        &[
            "pubkey",
            "runtime_generation",
            "last_distill_at",
            "activity",
        ],
        &["session_id"],
    )?;
    require_shape(
        conn,
        6,
        "session_locators",
        &["locator_value", "pubkey"],
        &[],
    )?;
    require_shape(
        conn,
        6,
        "llm_calls",
        &["pubkey", "window_hash"],
        &["session_id"],
    )?;
    let tx = conn.transaction().context("starting schema-6 migration")?;
    tx.execute_batch(
        r#"
        DROP TABLE llm_calls;
        ALTER TABLE sessions DROP COLUMN last_distill_at;
        ALTER TABLE sessions DROP COLUMN work_topic;
        ALTER TABLE sessions DROP COLUMN work_topic_set_at;
        ALTER TABLE sessions DROP COLUMN activity;
        ALTER TABLE sessions DROP COLUMN distill_fail_streak;
        ALTER TABLE sessions DROP COLUMN distill_notice_at;
        PRAGMA user_version = 7;
        "#,
    )?;
    tx.commit().context("committing schema-6 migration")
}

pub(super) fn v7_to_v8(conn: &mut Connection, path: &Path) -> Result<()> {
    require_shape(
        conn,
        7,
        "sessions",
        &["pubkey", "runtime_generation", "explicit_chat_published_at"],
        &["work_root", "readiness_parent", "last_distill_at"],
    )?;
    require_shape(
        conn,
        7,
        "outbox",
        &["event_json", "state", "next_attempt_at"],
        &[],
    )?;
    require_shape(conn, 7, "trellis_commits", &["transaction_id"], &[])?;
    require_shape(conn, 7, "trellis_replay_capsules", &["script_json"], &[])?;
    let pending = pending_outbox(conn)?;
    journal::merge_pending_writes(path, pending)?;
    let tx = conn.transaction().context("starting schema-7 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE sessions ADD COLUMN work_root TEXT NOT NULL DEFAULT '';
        ALTER TABLE sessions ADD COLUMN readiness_parent TEXT NOT NULL DEFAULT '';
        DROP TABLE outbox;
        DROP TABLE trellis_commits;
        DROP TABLE trellis_replay_capsules;
        PRAGMA user_version = 8;
        "#,
    )?;
    tx.commit().context("committing schema-7 migration")
}

pub(super) fn v8_to_v9(conn: &mut Connection, _path: &Path) -> Result<()> {
    require_shape(
        conn,
        8,
        "sessions",
        &["pubkey", "runtime_generation", "harness", "work_root"],
        &["observed_harness", "admitted_transport"],
    )?;
    let tx = conn.transaction().context("starting schema-8 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE sessions RENAME COLUMN harness TO observed_harness;
        ALTER TABLE sessions ADD COLUMN claimed_harness TEXT NOT NULL DEFAULT '';
        ALTER TABLE sessions ADD COLUMN admitted_bundle TEXT NOT NULL DEFAULT '';
        ALTER TABLE sessions ADD COLUMN admitted_transport TEXT NOT NULL DEFAULT ''
            CHECK (admitted_transport IN ('', 'pty', 'acp', 'app-server'));
        ALTER TABLE sessions ADD COLUMN endpoint_provenance TEXT NOT NULL DEFAULT ''
            CHECK (endpoint_provenance IN ('', 'launch', 'hook', 'migration'));
        ALTER TABLE session_locators RENAME TO session_locators_v8;
        CREATE TABLE session_locators (
            harness TEXT NOT NULL,
            locator_kind TEXT NOT NULL
                CHECK (locator_kind IN ('native_resume', 'pty', 'acp', 'app_server', 'pid')),
            locator_value TEXT NOT NULL,
            pubkey TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (harness, locator_kind, locator_value)
        );
        INSERT INTO session_locators
            (harness, locator_kind, locator_value, pubkey, created_at)
        SELECT harness,
               CASE WHEN harness='codex' AND locator_kind='acp'
                    THEN 'app_server' ELSE locator_kind END,
               locator_value, pubkey, created_at
        FROM session_locators_v8;
        DROP TABLE session_locators_v8;
        CREATE INDEX idx_session_locators_pubkey ON session_locators(pubkey);
        CREATE INDEX idx_session_locators_value ON session_locators(locator_value);
        CREATE UNIQUE INDEX idx_session_locators_native_resume
            ON session_locators(pubkey) WHERE locator_kind='native_resume';
        UPDATE sessions SET admitted_transport = CASE
            WHEN EXISTS (SELECT 1 FROM session_locators l
                         WHERE l.pubkey=sessions.pubkey
                           AND l.harness=sessions.observed_harness
                           AND l.locator_kind='app_server') THEN 'app-server'
            WHEN EXISTS (SELECT 1 FROM session_locators l
                         WHERE l.pubkey=sessions.pubkey
                           AND l.harness=sessions.observed_harness
                           AND l.locator_kind='acp') THEN 'acp'
            WHEN EXISTS (SELECT 1 FROM session_locators l
                         WHERE l.pubkey=sessions.pubkey
                           AND l.harness=sessions.observed_harness
                           AND l.locator_kind='pty') THEN 'pty'
            ELSE '' END;
        UPDATE sessions SET endpoint_provenance = 'migration';
        PRAGMA user_version = 9;
        "#,
    )?;
    tx.commit().context("committing schema-8 migration")
}

fn pending_outbox(conn: &Connection) -> Result<Vec<String>> {
    let mut statement =
        conn.prepare("SELECT event_json FROM outbox WHERE state='pending' ORDER BY local_id")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .context("collecting schema-7 pending writes")
}

fn require_shape(
    conn: &Connection,
    version: u32,
    table: &str,
    required: &[&str],
    forbidden: &[&str],
) -> Result<()> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    if columns.is_empty() {
        anyhow::bail!("schema {version} is missing table `{table}`");
    }
    for column in required {
        if !columns.contains(*column) {
            anyhow::bail!("schema {version} table `{table}` is missing `{column}`");
        }
    }
    for column in forbidden {
        if columns.contains(*column) {
            anyhow::bail!("schema {version} table `{table}` unexpectedly contains `{column}`");
        }
    }
    Ok(())
}
