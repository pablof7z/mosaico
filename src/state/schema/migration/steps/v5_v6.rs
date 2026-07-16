use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction};

use super::require_shape;

pub(super) fn migrate(conn: &mut Connection, _path: &Path) -> Result<()> {
    require_shape(
        conn,
        5,
        "sessions",
        &["session_id", "agent_pubkey"],
        &["pubkey"],
    )?;
    require_shape(
        conn,
        5,
        "session_aliases",
        &["external_id", "session_id"],
        &[],
    )?;
    require_shape(
        conn,
        5,
        "llm_calls",
        &["session_id", "window_hash"],
        &["pubkey"],
    )?;
    require_shape(conn, 5, "messages", &["author_pubkey"], &["author_session"])?;
    let tx = conn.transaction().context("starting schema-5 migration")?;
    migrate_session_identity(&tx)?;
    tx.commit().context("committing schema-5 migration")
}

fn migrate_session_identity(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_sessions_alive;
        DROP INDEX IF EXISTS idx_session_channels_channel;
        DROP INDEX IF EXISTS idx_session_aliases_session;
        DROP INDEX IF EXISTS idx_session_aliases_external;
        DROP INDEX IF EXISTS idx_session_claims_expires;
        DROP INDEX IF EXISTS idx_session_claims_session;
        DROP INDEX IF EXISTS idx_llm_calls_session;
        DROP INDEX IF EXISTS idx_llm_calls_window_hash;
        DROP TABLE IF EXISTS relay_channels;
        DROP TABLE IF EXISTS relay_channel_members;
        DROP TABLE IF EXISTS relay_channel_member_sets;
        DROP TABLE IF EXISTS relay_profiles;
        DROP TABLE IF EXISTS relay_status;
        DROP TABLE IF EXISTS relay_agent_roster;
        DROP TABLE IF EXISTS relay_events;
        DROP TABLE IF EXISTS relay_reactions;
        DROP TABLE IF EXISTS relay_event_quarantine;
        DROP TABLE IF EXISTS identities;
        DROP TABLE IF EXISTS durable_agent_sessions;
        ALTER TABLE sessions RENAME TO migration_v5_sessions;
        ALTER TABLE session_channels RENAME TO migration_v5_session_channels;
        ALTER TABLE session_aliases RENAME TO migration_v5_session_aliases;
        ALTER TABLE session_claims RENAME TO migration_v5_session_claims;
        ALTER TABLE llm_calls RENAME TO migration_v5_llm_calls;

        CREATE TABLE sessions (
            pubkey TEXT PRIMARY KEY, runtime_generation INTEGER NOT NULL,
            agent_slug TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '', child_pid INTEGER, transcript_path TEXT,
            alive INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
            last_seen INTEGER NOT NULL DEFAULT 0, working INTEGER NOT NULL DEFAULT 0,
            turn_started_at INTEGER NOT NULL DEFAULT 0,
            last_distill_at INTEGER NOT NULL DEFAULT 0, work_topic TEXT NOT NULL DEFAULT '',
            work_topic_set_at INTEGER NOT NULL DEFAULT 0, seen_cursor INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '', activity TEXT NOT NULL DEFAULT '',
            distill_fail_streak INTEGER NOT NULL DEFAULT 0,
            distill_notice_at INTEGER NOT NULL DEFAULT 0,
            explicit_chat_published_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX idx_sessions_alive ON sessions(alive, channel_h);
        CREATE TABLE session_channels (
            pubkey TEXT NOT NULL, channel_h TEXT NOT NULL, joined_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_channels_channel ON session_channels(channel_h, pubkey);
        CREATE TABLE session_locators (
            harness TEXT NOT NULL,
            locator_kind TEXT NOT NULL CHECK (locator_kind IN ('native_resume', 'pty', 'acp', 'pid')),
            locator_value TEXT NOT NULL, pubkey TEXT NOT NULL, created_at INTEGER NOT NULL,
            PRIMARY KEY (harness, locator_kind, locator_value)
        );
        CREATE INDEX idx_session_locators_pubkey ON session_locators(pubkey);
        CREATE INDEX idx_session_locators_value ON session_locators(locator_value);
        CREATE UNIQUE INDEX idx_session_locators_native_resume
            ON session_locators(pubkey) WHERE locator_kind='native_resume';
        CREATE TABLE session_claims (
            pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL DEFAULT '', harness TEXT NOT NULL DEFAULT '',
            last_active_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            owner_backend_pubkey TEXT NOT NULL DEFAULT '', owner_host TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_claims_expires ON session_claims(expires_at);
        CREATE TABLE llm_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT, pubkey TEXT NOT NULL,
            window_hash TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
            system_prompt TEXT NOT NULL, transcript_slice TEXT NOT NULL,
            raw_response TEXT NOT NULL, parsed_title TEXT, parsed_activity TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX idx_llm_calls_pubkey ON llm_calls(pubkey, created_at);
        CREATE INDEX idx_llm_calls_window_hash ON llm_calls(window_hash);

        INSERT OR REPLACE INTO sessions
        SELECT agent_pubkey, 0, agent_slug, channel_h, harness, child_pid,
               transcript_path, alive, created_at, last_seen, working, turn_started_at,
               last_distill_at, work_topic, work_topic_set_at, seen_cursor, title, activity,
               distill_fail_streak, distill_notice_at, explicit_chat_published_at
          FROM migration_v5_sessions ORDER BY last_seen;
        INSERT OR IGNORE INTO session_channels
        SELECT s.agent_pubkey, c.channel_h, c.joined_at
          FROM migration_v5_session_channels c
          JOIN migration_v5_sessions s USING (session_id);
        INSERT OR IGNORE INTO session_locators
        SELECT a.harness,
               CASE a.external_id_kind WHEN 'harness_session' THEN 'native_resume'
                   WHEN 'pty_session' THEN 'pty' WHEN 'watch_pid' THEN 'pid' ELSE 'acp' END,
               a.external_id, s.agent_pubkey, a.created_at
          FROM migration_v5_session_aliases a
          JOIN migration_v5_sessions s USING (session_id);
        INSERT OR REPLACE INTO session_claims
        SELECT pubkey, agent_slug, channel_h, harness, last_active_at, expires_at,
               owner_backend_pubkey, owner_host FROM migration_v5_session_claims;
        INSERT INTO llm_calls
        SELECT l.id, s.agent_pubkey, l.window_hash, l.provider, l.model, l.system_prompt,
               l.transcript_slice, l.raw_response, l.parsed_title, l.parsed_activity, l.created_at
          FROM migration_v5_llm_calls l
          JOIN migration_v5_sessions s USING (session_id);
        DROP TABLE migration_v5_sessions;
        DROP TABLE migration_v5_session_channels;
        DROP TABLE migration_v5_session_aliases;
        DROP TABLE migration_v5_session_claims;
        DROP TABLE migration_v5_llm_calls;
        PRAGMA user_version = 6;
        "#,
    )?;
    Ok(())
}
