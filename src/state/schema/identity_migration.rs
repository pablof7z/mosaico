use anyhow::{Context, Result};
use rusqlite::Connection;

/// Reshape a legacy `identities` table to the per-session-pubkey schema: drop
/// `base_pubkey` + `ordinal`, add `codename`. `identities` is local state (it maps
/// a minted pubkey back to its live session for resume), so this is a real
/// rebuild that PRESERVES existing rows, backfilling each row's codename from its
/// session id — never a drop.
pub(super) fn ensure_session_primary_key(conn: &Connection) -> Result<()> {
    let columns = table_columns(conn, "identities").context("reading identities schema")?;
    if columns.is_empty() {
        // Fresh DB: the DDL already created the canonical shape.
        return Ok(());
    }
    // Already migrated: has codename and no legacy base_pubkey.
    if columns.iter().any(|c| c == "codename") && !columns.iter().any(|c| c == "base_pubkey") {
        return Ok(());
    }

    #[derive(Default)]
    struct Row {
        pubkey: String,
        agent_slug: String,
        session_id: String,
        channel_h: String,
        native_id: String,
        alive: i64,
        created_at: i64,
    }

    let rows: Vec<Row> = {
        let mut stmt = conn
            .prepare(
                "SELECT pubkey, agent_slug, session_id, channel_h, native_id, alive, created_at \
                 FROM identities",
            )
            .context("reading legacy identities rows")?;
        let mapped = stmt.query_map([], |r| {
            Ok(Row {
                pubkey: r.get(0)?,
                agent_slug: r.get(1)?,
                session_id: r.get(2)?,
                channel_h: r.get(3)?,
                native_id: r.get(4)?,
                alive: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?;
        mapped
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting legacy identities rows")?
    };

    conn.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_identities_base;
        DROP INDEX IF EXISTS idx_identities_channel;
        DROP INDEX IF EXISTS idx_identities_session;
        DROP TABLE identities;
        CREATE TABLE identities (
            pubkey       TEXT NOT NULL,
            agent_slug   TEXT NOT NULL DEFAULT '',
            codename     TEXT NOT NULL DEFAULT '',
            session_id   TEXT NOT NULL DEFAULT '',
            channel_h    TEXT NOT NULL DEFAULT '',
            native_id    TEXT NOT NULL DEFAULT '',
            alive        INTEGER NOT NULL DEFAULT 0,
            created_at   INTEGER NOT NULL,
            PRIMARY KEY (pubkey, session_id)
        );
        CREATE INDEX IF NOT EXISTS idx_identities_channel
            ON identities(channel_h);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_identities_session
            ON identities(session_id) WHERE session_id <> '';
        "#,
    )
    .context("rebuilding identities table for per-session codename schema")?;

    for row in rows {
        let codename = crate::util::friendly_short_code(&row.session_id);
        conn.execute(
            "INSERT OR REPLACE INTO identities
                 (pubkey, agent_slug, codename, session_id, channel_h, native_id, alive, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                row.pubkey,
                row.agent_slug,
                codename,
                row.session_id,
                row.channel_h,
                row.native_id,
                row.alive,
                row.created_at,
            ],
        )
        .context("backfilling migrated identity row")?;
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(cols)
}
