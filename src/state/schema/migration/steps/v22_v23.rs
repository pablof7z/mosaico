//! Schema 23: admit Pi's native RPC transport and runtime locator.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub(in crate::state::schema::migration) fn migrate(
    conn: &mut Connection,
    _path: &Path,
) -> Result<()> {
    let tx = conn.transaction().context("starting schema-23 migration")?;
    tx.execute_batch(
        r#"
        ALTER TABLE sessions RENAME TO sessions_v22;
        CREATE TABLE sessions (
            pubkey TEXT PRIMARY KEY,
            runtime_generation INTEGER NOT NULL,
            agent_slug TEXT NOT NULL DEFAULT '',
            work_root TEXT NOT NULL DEFAULT '',
            readiness_parent TEXT NOT NULL DEFAULT '',
            observed_harness TEXT NOT NULL DEFAULT '',
            claimed_harness TEXT NOT NULL DEFAULT '',
            admitted_bundle TEXT NOT NULL DEFAULT '',
            admitted_transport TEXT NOT NULL DEFAULT ''
                CHECK (admitted_transport IN ('', 'pty', 'acp', 'app-server', 'pi-rpc')),
            endpoint_provenance TEXT NOT NULL DEFAULT ''
                CHECK (endpoint_provenance IN ('', 'launch', 'hook', 'migration')),
            child_pid INTEGER,
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
            busy_seconds INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            last_seen INTEGER NOT NULL DEFAULT 0,
            turn_started_at INTEGER NOT NULL DEFAULT 0,
            seen_cursor INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '',
            state_changed_at INTEGER NOT NULL DEFAULT 0
        );
        INSERT INTO sessions (
            pubkey, runtime_generation, agent_slug, work_root, readiness_parent,
            observed_harness, claimed_harness, admitted_bundle,
            admitted_transport, endpoint_provenance, child_pid, runtime_state,
            presentation_state, work_state, recovery_state, lifecycle_epoch,
            attachment_epoch, idle_since, idle_deadline, stopped_at, stop_reason,
            turn_count, busy_seconds, created_at, last_seen, turn_started_at,
            seen_cursor, title, state_changed_at
        )
        SELECT
            pubkey, runtime_generation, agent_slug, work_root, readiness_parent,
            observed_harness, claimed_harness, admitted_bundle,
            admitted_transport, endpoint_provenance, child_pid, runtime_state,
            presentation_state, work_state, recovery_state, lifecycle_epoch,
            attachment_epoch, idle_since, idle_deadline, stopped_at, stop_reason,
            turn_count, busy_seconds, created_at, last_seen, turn_started_at,
            seen_cursor, title, state_changed_at
        FROM sessions_v22;
        DROP TABLE sessions_v22;
        CREATE INDEX idx_sessions_runtime ON sessions(runtime_state);
        CREATE INDEX idx_sessions_idle_deadline
            ON sessions(runtime_state, presentation_state, work_state, idle_deadline);

        ALTER TABLE session_locators RENAME TO session_locators_v22;
        CREATE TABLE session_locators (
            harness TEXT NOT NULL,
            locator_kind TEXT NOT NULL CHECK (locator_kind IN (
                'native_resume', 'pty', 'acp', 'app_server', 'pi_rpc', 'pid'
            )),
            locator_value TEXT NOT NULL,
            pubkey TEXT NOT NULL,
            runtime_generation INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            PRIMARY KEY (harness, locator_kind, locator_value)
        );
        INSERT INTO session_locators (
            harness, locator_kind, locator_value, pubkey,
            runtime_generation, created_at
        )
        SELECT harness, locator_kind, locator_value, pubkey,
               runtime_generation, created_at
          FROM session_locators_v22;
        DROP TABLE session_locators_v22;
        CREATE INDEX idx_session_locators_pubkey ON session_locators(pubkey);
        CREATE INDEX idx_session_locators_value ON session_locators(locator_value);
        CREATE UNIQUE INDEX idx_session_locators_native_resume
            ON session_locators(pubkey) WHERE locator_kind='native_resume';
        CREATE UNIQUE INDEX idx_session_locators_runtime_endpoint
            ON session_locators(pubkey, harness, locator_kind)
            WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pi_rpc', 'pid');
        PRAGMA user_version = 23;
        "#,
    )?;
    tx.commit().context("committing schema-23 migration")
}
