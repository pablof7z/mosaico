use std::path::Path;

use rusqlite::Connection;

#[path = "migration_fixture/v17.rs"]
mod v17;
pub(super) use v17::downgrade_channel_context_to_v17;
#[path = "migration_fixture/group_v21.rs"]
mod group_v21;
#[path = "migration_fixture/v4.rs"]
mod v4;
pub(super) use group_v21::{
    restore_relay_derived_tables, restore_relay_group_tables, restore_relay_message_tables,
};

fn create_current(conn: &Connection) {
    for part in super::super::super::ddl::SCHEMA_PARTS {
        conn.execute_batch(part).unwrap();
    }
    restore_relay_derived_tables(conn);
    conn.execute("DROP TABLE session_coaching", []).unwrap();
}

fn downgrade_launch_admission_to_v22(conn: &Connection) {
    let has_preset = conn
        .prepare("PRAGMA table_info(sessions)")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .any(|name| name.is_ok_and(|name| name == "admitted_preset"));
    if has_preset {
        conn.execute_batch(
            "ALTER TABLE sessions DROP COLUMN admitted_preset;
             ALTER TABLE sessions ADD COLUMN admitted_bundle TEXT NOT NULL DEFAULT '';",
        )
        .unwrap();
    }
}

pub(super) fn create_schema_four(path: &Path) {
    v4::create_schema_four(path);
}

/// Restore `messages.direction`, which schema 20 deleted.
///
/// Every fixture below builds an old schema by starting from the CURRENT DDL
/// and reverting what changed since, so a column deleted at 20 has to come back
/// before a database can honestly claim to be stamped 19 or earlier.
pub(super) fn downgrade_messages_to_v19(conn: &Connection) {
    downgrade_launch_admission_to_v22(conn);
    restore_relay_group_tables(conn);
    restore_relay_message_tables(conn);
    conn.execute("DROP TABLE session_coaching", []).unwrap();
    conn.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_messages_author_pubkey;
        ALTER TABLE messages ADD COLUMN direction TEXT NOT NULL DEFAULT 'inbound';
        CREATE INDEX idx_messages_author_pubkey
            ON messages(author_pubkey, direction, sync_state, created_at);
        "#,
    )
    .unwrap();
}

pub(super) fn add_removed_v15_session_columns(conn: &Connection) {
    conn.execute_batch(
        r#"
        ALTER TABLE sessions ADD COLUMN transcript_path TEXT;
        ALTER TABLE sessions
            ADD COLUMN explicit_chat_published_at INTEGER NOT NULL DEFAULT 0;
        "#,
    )
    .unwrap();
}

pub(super) fn create_schema_seven(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_current(&conn);
    downgrade_channel_context_to_v17(&conn);
    add_removed_v15_session_columns(&conn);
    conn.execute_batch(
        r#"
        ALTER TABLE sessions DROP COLUMN work_root;
        ALTER TABLE sessions DROP COLUMN readiness_parent;
        CREATE TABLE outbox (
            local_id INTEGER PRIMARY KEY AUTOINCREMENT, event_json TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending', retries INTEGER NOT NULL DEFAULT 0,
            last_error TEXT, enqueued_at INTEGER NOT NULL,
            next_attempt_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE trellis_commits (
            id INTEGER PRIMARY KEY AUTOINCREMENT, transaction_id INTEGER NOT NULL
        );
        CREATE TABLE trellis_replay_capsules (
            id INTEGER PRIMARY KEY AUTOINCREMENT, script_json TEXT NOT NULL
        );
        PRAGMA user_version = 7;
        "#,
    )
    .unwrap();
}

pub(super) fn create_schema_eight(path: &Path) {
    let conn = Connection::open(path).unwrap();
    create_current(&conn);
    downgrade_channel_context_to_v17(&conn);
    add_removed_v15_session_columns(&conn);
    conn.execute_batch(
        r#"
        DROP INDEX idx_sessions_runtime;
        DROP INDEX idx_messages_author_pubkey;
        ALTER TABLE messages ADD COLUMN direction TEXT NOT NULL DEFAULT 'inbound';
        CREATE INDEX idx_messages_author_pubkey
            ON messages(author_pubkey, direction, sync_state, created_at);
        DROP INDEX idx_sessions_idle_deadline;
        DROP INDEX idx_session_locators_runtime_endpoint;
        DROP INDEX idx_session_channels_channel;
        DROP INDEX idx_session_standing_due;
        DROP TABLE session_standing;
        DROP TABLE session_channels;
        CREATE TABLE session_channels (
            pubkey TEXT NOT NULL, channel_h TEXT NOT NULL, joined_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_channels_channel ON session_channels(channel_h, pubkey);
        CREATE TABLE session_claims (
            pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL DEFAULT '', harness TEXT NOT NULL DEFAULT '',
            last_active_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            owner_backend_pubkey TEXT NOT NULL DEFAULT '', owner_host TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_claims_expires ON session_claims(expires_at);
        ALTER TABLE session_locators DROP COLUMN runtime_generation;
        ALTER TABLE sessions DROP COLUMN runtime_state;
        ALTER TABLE sessions DROP COLUMN presentation_state;
        ALTER TABLE sessions DROP COLUMN work_state;
        ALTER TABLE sessions DROP COLUMN recovery_state;
        ALTER TABLE sessions DROP COLUMN lifecycle_epoch;
        ALTER TABLE sessions DROP COLUMN attachment_epoch;
        ALTER TABLE sessions DROP COLUMN idle_since;
        ALTER TABLE sessions DROP COLUMN idle_deadline;
        ALTER TABLE sessions DROP COLUMN stopped_at;
        ALTER TABLE sessions DROP COLUMN stop_reason;
        ALTER TABLE sessions DROP COLUMN turn_count;
        ALTER TABLE sessions ADD COLUMN alive INTEGER NOT NULL DEFAULT 1;
        ALTER TABLE sessions ADD COLUMN working INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE sessions DROP COLUMN claimed_harness;
        ALTER TABLE sessions DROP COLUMN admitted_bundle;
        ALTER TABLE sessions DROP COLUMN admitted_transport;
        ALTER TABLE sessions DROP COLUMN endpoint_provenance;
        ALTER TABLE sessions RENAME COLUMN observed_harness TO harness;
        INSERT INTO sessions
            (pubkey, runtime_generation, harness, created_at)
        VALUES ('pk-pty', 1, 'codex', 1),
               ('pk-acp', 1, 'claude-code', 1),
               ('pk-app-server', 1, 'codex', 1);
        INSERT INTO session_locators
            (harness, locator_kind, locator_value, pubkey, created_at)
        VALUES ('codex', 'pty', 'pty-owned', 'pk-pty', 1),
               ('claude-code', 'acp', 'acp-foreign', 'pk-pty', 2),
               ('claude-code', 'acp', 'acp-owned', 'pk-acp', 1),
               ('codex', 'acp', 'app-server-owned', 'pk-app-server', 1);
        PRAGMA user_version = 8;
        "#,
    )
    .unwrap();
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}
