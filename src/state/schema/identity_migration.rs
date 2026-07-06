use anyhow::{Context, Result};
use rusqlite::Connection;

pub(super) fn ensure_session_primary_key(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(identities)")
        .context("reading identities schema")?;
    let columns = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("collecting identities schema")?;
    let pk_cols: Vec<&str> = columns
        .iter()
        .filter(|(_, pk)| *pk > 0)
        .map(|(name, _)| name.as_str())
        .collect();
    if pk_cols == ["pubkey", "session_id"] {
        return Ok(());
    }
    if pk_cols != ["pubkey"] {
        return Ok(());
    }

    conn.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_identities_base;
        DROP INDEX IF EXISTS idx_identities_session;
        ALTER TABLE identities RENAME TO identities_pubkey_pk_legacy;
        CREATE TABLE identities (
            pubkey       TEXT NOT NULL,
            base_pubkey  TEXT NOT NULL,
            agent_slug   TEXT NOT NULL DEFAULT '',
            ordinal      INTEGER NOT NULL DEFAULT 0,
            session_id   TEXT NOT NULL DEFAULT '',
            channel_h    TEXT NOT NULL DEFAULT '',
            native_id    TEXT NOT NULL DEFAULT '',
            alive        INTEGER NOT NULL DEFAULT 0,
            created_at   INTEGER NOT NULL,
            PRIMARY KEY (pubkey, session_id)
        );
        INSERT OR REPLACE INTO identities
            (pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h,
             native_id, alive, created_at)
        SELECT pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h,
               native_id, alive, created_at
          FROM identities_pubkey_pk_legacy;
        DROP TABLE identities_pubkey_pk_legacy;
        CREATE INDEX IF NOT EXISTS idx_identities_base
            ON identities(base_pubkey, channel_h);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_identities_session
            ON identities(session_id) WHERE session_id <> '';
        "#,
    )
    .context("migrating identities primary key to session-scoped rows")?;
    Ok(())
}
