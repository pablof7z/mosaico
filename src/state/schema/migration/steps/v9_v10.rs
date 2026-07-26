use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::require_shape;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    require_shape(
        conn,
        9,
        "sessions",
        &[
            "pubkey",
            "runtime_generation",
            "observed_harness",
            "claimed_harness",
            "admitted_bundle",
            "admitted_transport",
            "endpoint_provenance",
            "alive",
            "working",
        ],
        &["runtime_state", "lifecycle_epoch", "turn_count", "harness"],
    )?;
    require_shape(
        conn,
        9,
        "session_channels",
        &["pubkey", "channel_h", "joined_at"],
        &["granted_at"],
    )?;
    require_shape(
        conn,
        9,
        "session_claims",
        &["pubkey", "channel_h", "expires_at"],
        &[],
    )?;
    require_shape(
        conn,
        9,
        "session_locators",
        &[
            "harness",
            "locator_kind",
            "locator_value",
            "pubkey",
            "created_at",
        ],
        &["runtime_generation"],
    )?;
    let tx = conn.transaction().context("starting schema-9 migration")?;
    tx.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_sessions_alive;
        DROP INDEX IF EXISTS idx_session_channels_channel;
        DROP INDEX IF EXISTS idx_session_claims_expires;
        ALTER TABLE sessions RENAME TO migration_v9_sessions;
        ALTER TABLE session_channels RENAME TO migration_v9_session_channels;
        DROP TABLE session_claims;

        CREATE TABLE IF NOT EXISTS relay_channel_members (
            channel_h TEXT NOT NULL, pubkey TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member', updated_at INTEGER NOT NULL,
            PRIMARY KEY (channel_h, pubkey)
        );
        CREATE TABLE IF NOT EXISTS relay_channels (
            channel_h TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            about TEXT NOT NULL DEFAULT '',
            parent TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(parent, name)
        );
        CREATE TABLE IF NOT EXISTS relay_status (
            pubkey TEXT NOT NULL,
            channel_h TEXT NOT NULL,
            slug TEXT NOT NULL DEFAULT '',
            title TEXT NOT NULL DEFAULT '',
            activity TEXT NOT NULL DEFAULT '',
            state TEXT NOT NULL,
            state_since INTEGER NOT NULL DEFAULT 0,
            last_seen INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0,
            expiration INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE TABLE IF NOT EXISTS relay_events (
            id TEXT PRIMARY KEY,
            kind INTEGER NOT NULL,
            pubkey TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            channel_h TEXT NOT NULL DEFAULT '',
            d_tag TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL DEFAULT '',
            tags_json TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE sessions (
            pubkey TEXT PRIMARY KEY, runtime_generation INTEGER NOT NULL,
            agent_slug TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL DEFAULT '',
            work_root TEXT NOT NULL DEFAULT '', readiness_parent TEXT NOT NULL DEFAULT '',
            observed_harness TEXT NOT NULL DEFAULT '', claimed_harness TEXT NOT NULL DEFAULT '',
            admitted_bundle TEXT NOT NULL DEFAULT '',
            admitted_transport TEXT NOT NULL DEFAULT ''
                CHECK (admitted_transport IN ('', 'pty', 'acp', 'app-server')),
            endpoint_provenance TEXT NOT NULL DEFAULT ''
                CHECK (endpoint_provenance IN ('', 'launch', 'hook', 'migration')),
            child_pid INTEGER, transcript_path TEXT,
            runtime_state TEXT NOT NULL DEFAULT 'running'
                CHECK (runtime_state IN ('running', 'stopping', 'stopped')),
            presentation_state TEXT NOT NULL DEFAULT 'unavailable'
                CHECK (presentation_state IN ('unavailable', 'headed', 'headless')),
            work_state TEXT NOT NULL DEFAULT 'idle'
                CHECK (work_state IN ('idle', 'working')),
            recovery_state TEXT NOT NULL DEFAULT 'pending'
                CHECK (recovery_state IN ('pending', 'ready', 'revoked')),
            lifecycle_epoch INTEGER NOT NULL DEFAULT 1,
            attachment_epoch INTEGER NOT NULL DEFAULT 0,
            idle_since INTEGER NOT NULL DEFAULT 0,
            idle_deadline INTEGER NOT NULL DEFAULT 0,
            stopped_at INTEGER NOT NULL DEFAULT 0,
            stop_reason TEXT CHECK (stop_reason IS NULL OR stop_reason IN (
                'unknown', 'attached_clean_exit', 'idle_evicted', 'headless_exit',
                'crash', 'operator_kill', 'revoked', 'superseded'
            )),
            turn_count INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL, last_seen INTEGER NOT NULL DEFAULT 0,
            turn_started_at INTEGER NOT NULL DEFAULT 0,
            seen_cursor INTEGER NOT NULL DEFAULT 0, title TEXT NOT NULL DEFAULT '',
            explicit_chat_published_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX idx_sessions_runtime ON sessions(runtime_state, channel_h);
        CREATE INDEX idx_sessions_idle_deadline
            ON sessions(runtime_state, presentation_state, work_state, idle_deadline);

        INSERT INTO sessions
        SELECT old.pubkey, old.runtime_generation, old.agent_slug, old.channel_h,
               old.work_root, old.readiness_parent,
               old.observed_harness, old.claimed_harness, old.admitted_bundle,
               old.admitted_transport, old.endpoint_provenance,
               old.child_pid, old.transcript_path,
               CASE old.alive WHEN 1 THEN 'running' ELSE 'stopped' END,
               'unavailable',
               CASE old.working WHEN 1 THEN 'working' ELSE 'idle' END,
               CASE WHEN EXISTS (
                   SELECT 1 FROM session_locators locator
                   WHERE locator.pubkey=old.pubkey AND locator.locator_kind='native_resume'
               ) THEN 'ready' ELSE 'pending' END,
               CASE WHEN old.runtime_generation > 0 THEN old.runtime_generation ELSE 1 END,
               0, 0, 0,
               CASE old.alive WHEN 1 THEN 0 ELSE old.last_seen END,
               CASE old.alive WHEN 1 THEN NULL ELSE 'unknown' END,
               CASE WHEN old.working=1 OR old.turn_started_at>0 OR old.seen_cursor>0
                          OR old.explicit_chat_published_at>0 OR EXISTS (
                       SELECT 1 FROM session_locators locator
                       WHERE locator.pubkey=old.pubkey
                         AND locator.locator_kind='native_resume'
                   ) THEN 1 ELSE 0 END,
               old.created_at, old.last_seen, old.turn_started_at, old.seen_cursor,
               old.title, old.explicit_chat_published_at
          FROM migration_v9_sessions old;

        ALTER TABLE session_locators
            ADD COLUMN runtime_generation INTEGER NOT NULL DEFAULT 0;
        UPDATE session_locators
           SET runtime_generation=COALESCE(
               (SELECT runtime_generation FROM sessions
                 WHERE sessions.pubkey=session_locators.pubkey), 0)
         WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pid');
        DELETE FROM session_locators
         WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pid')
           AND rowid NOT IN (
               SELECT MAX(rowid) FROM session_locators
                WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pid')
                GROUP BY pubkey, harness, locator_kind
           );
        CREATE UNIQUE INDEX idx_session_locators_runtime_endpoint
            ON session_locators(pubkey, harness, locator_kind)
            WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pid');

        CREATE TABLE session_channels (
            pubkey TEXT NOT NULL, channel_h TEXT NOT NULL, granted_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_channels_channel ON session_channels(channel_h, pubkey);
        INSERT OR IGNORE INTO session_channels
        SELECT pubkey, channel_h, joined_at FROM migration_v9_session_channels;
        INSERT OR IGNORE INTO session_channels
        SELECT pubkey, channel_h, created_at FROM sessions WHERE channel_h<>'';

        CREATE TABLE session_standing (
            pubkey TEXT NOT NULL, channel_h TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('member', 'retained', 'absent')),
            retain_until INTEGER NOT NULL DEFAULT 0,
            standing_epoch INTEGER NOT NULL DEFAULT 1,
            session_lifecycle_epoch INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_standing_due ON session_standing(state, retain_until);
        INSERT INTO session_standing
        SELECT route.pubkey, route.channel_h,
               CASE WHEN member.pubkey IS NULL THEN 'absent'
                    WHEN session.runtime_state='stopped' THEN 'retained'
                    ELSE 'member' END,
               CASE WHEN member.pubkey IS NOT NULL AND session.runtime_state='stopped'
                    THEN session.last_seen + 3600 ELSE 0 END,
               1,
               session.lifecycle_epoch, MAX(session.last_seen, route.granted_at)
          FROM session_channels route
          JOIN sessions session USING (pubkey)
          LEFT JOIN relay_channel_members member
            ON member.pubkey=route.pubkey AND member.channel_h=route.channel_h;

        DROP TABLE migration_v9_sessions;
        DROP TABLE migration_v9_session_channels;
        PRAGMA user_version = 10;
        "#,
    )?;
    tx.commit().context("committing schema-9 migration")
}
