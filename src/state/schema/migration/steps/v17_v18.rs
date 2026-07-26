use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    super::require_shape(
        conn,
        17,
        "sessions",
        &[
            "pubkey",
            "channel_h",
            "work_root",
            "runtime_state",
            "created_at",
        ],
        &[],
    )?;
    super::require_shape(
        conn,
        17,
        "session_channels",
        &["pubkey", "channel_h", "granted_at"],
        &["joined_at", "joined_event_seq"],
    )?;
    super::require_shape(
        conn,
        17,
        "relay_channels",
        &[
            "channel_h",
            "name",
            "about",
            "parent",
            "created_at",
            "updated_at",
        ],
        &[],
    )?;
    super::require_shape(
        conn,
        17,
        "relay_status",
        &["pubkey", "channel_h", "state"],
        &["workspace", "branch"],
    )?;
    super::require_shape(
        conn,
        17,
        "session_standing",
        &[
            "pubkey",
            "channel_h",
            "state",
            "retain_until",
            "standing_epoch",
            "session_lifecycle_epoch",
            "updated_at",
        ],
        &[],
    )?;

    let tx = conn.transaction().context("starting schema-17 migration")?;
    tx.execute_batch(
        r#"
        -- Every former current-channel pointer becomes a durable membership.
        -- Existing rows win so their original join timestamp is preserved.
        INSERT INTO session_channels (pubkey, channel_h, granted_at)
        SELECT pubkey, channel_h, created_at
          FROM sessions
         WHERE channel_h<>''
        ON CONFLICT(pubkey, channel_h) DO NOTHING;

        DROP INDEX idx_session_channels_channel;
        ALTER TABLE session_channels RENAME TO migration_v17_session_channels;
        CREATE TABLE session_channels (
            pubkey            TEXT NOT NULL,
            channel_h         TEXT NOT NULL,
            joined_at         INTEGER NOT NULL,
            joined_event_seq  INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        INSERT INTO session_channels
            (pubkey, channel_h, joined_at, joined_event_seq)
        SELECT pubkey, channel_h, granted_at,
               (SELECT COALESCE(MAX(rowid), 0) FROM relay_events)
          FROM migration_v17_session_channels;
        DROP TABLE migration_v17_session_channels;
        CREATE INDEX idx_session_channels_channel
            ON session_channels(channel_h, pubkey);

        -- Schema 18 has only durable member/absent standing. Historical
        -- retained rows become members: ordinary runtime stop is not leave.
        DROP INDEX idx_session_standing_due;
        ALTER TABLE session_standing RENAME TO migration_v17_session_standing;
        CREATE TABLE session_standing (
            pubkey                  TEXT NOT NULL,
            channel_h               TEXT NOT NULL,
            state                   TEXT NOT NULL CHECK (state IN ('member', 'absent')),
            standing_epoch          INTEGER NOT NULL DEFAULT 1,
            session_lifecycle_epoch INTEGER NOT NULL,
            updated_at              INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        INSERT INTO session_standing
            (pubkey, channel_h, state, standing_epoch,
             session_lifecycle_epoch, updated_at)
        SELECT pubkey, channel_h,
               CASE WHEN state='absent' THEN 'absent' ELSE 'member' END,
               standing_epoch, session_lifecycle_epoch, updated_at
          FROM migration_v17_session_standing;
        DROP TABLE migration_v17_session_standing;
        CREATE INDEX idx_session_standing_state
            ON session_standing(state, pubkey, channel_h);

        DROP INDEX idx_sessions_runtime;
        ALTER TABLE sessions DROP COLUMN channel_h;
        CREATE INDEX idx_sessions_runtime ON sessions(runtime_state);

        ALTER TABLE relay_channels RENAME TO migration_v17_relay_channels;
        CREATE TABLE relay_channels (
            channel_h   TEXT PRIMARY KEY,
            name        TEXT NOT NULL DEFAULT '',
            about       TEXT NOT NULL DEFAULT '',
            parent      TEXT NOT NULL DEFAULT '',
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL
        );
        INSERT INTO relay_channels
            (channel_h, name, about, parent, created_at, updated_at)
        SELECT channel_h, name, about, parent, created_at, updated_at
          FROM migration_v17_relay_channels;
        DROP TABLE migration_v17_relay_channels;
        CREATE UNIQUE INDEX idx_relay_channels_named_sibling
            ON relay_channels(parent, name) WHERE parent<>'' AND name<>'';

        ALTER TABLE relay_status
            ADD COLUMN workspace TEXT NOT NULL DEFAULT '';
        ALTER TABLE relay_status
            ADD COLUMN branch TEXT NOT NULL DEFAULT '';
        CREATE TABLE relay_status_sets (
            pubkey TEXT PRIMARY KEY,
            updated_at INTEGER NOT NULL
        );
        INSERT INTO relay_status_sets (pubkey, updated_at)
        SELECT pubkey, MAX(updated_at)
          FROM relay_status
         GROUP BY pubkey;

        PRAGMA user_version = 18;
        "#,
    )?;
    tx.commit().context("committing schema-17 migration")
}
