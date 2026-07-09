use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;

const OWNER_COLUMNS: &[(&str, &str)] = &[
    ("owner_backend_pubkey", "TEXT NOT NULL DEFAULT ''"),
    ("owner_host", "TEXT NOT NULL DEFAULT ''"),
];

pub(super) fn ensure_columns(conn: &Connection) -> Result<()> {
    let existing =
        table_columns(conn, "session_claims").context("reading session_claims columns")?;
    if existing.is_empty() {
        // Fresh DB: the DDL already created the canonical shape.
        return Ok(());
    }

    // Legacy shape carried base_pubkey + ordinal — reshape to per-session codename.
    if existing.contains("base_pubkey") || existing.contains("ordinal") {
        reshape_legacy(conn, &existing)?;
        return Ok(());
    }

    // New-shape table that predates the owner columns: add them in place.
    for (name, spec) in OWNER_COLUMNS {
        if !existing.contains(*name) {
            conn.execute(
                &format!("ALTER TABLE session_claims ADD COLUMN {name} {spec}"),
                [],
            )
            .with_context(|| format!("adding session_claims.{name}"))?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Row {
    pubkey: String,
    agent_slug: String,
    session_id: String,
    channel_h: String,
    native_id: String,
    harness: String,
    last_active_at: i64,
    expires_at: i64,
    owner_backend_pubkey: String,
    owner_host: String,
}

fn reshape_legacy(conn: &Connection, existing: &BTreeSet<String>) -> Result<()> {
    let has_owner = existing.contains("owner_backend_pubkey");
    let owner_select = if has_owner {
        "owner_backend_pubkey, owner_host"
    } else {
        "'' AS owner_backend_pubkey, '' AS owner_host"
    };
    let rows: Vec<Row> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT pubkey, agent_slug, session_id, channel_h, native_id, harness, \
                 last_active_at, expires_at, {owner_select} FROM session_claims"
            ))
            .context("reading legacy session_claims rows")?;
        let mapped = stmt.query_map([], |r| {
            Ok(Row {
                pubkey: r.get(0)?,
                agent_slug: r.get(1)?,
                session_id: r.get(2)?,
                channel_h: r.get(3)?,
                native_id: r.get(4)?,
                harness: r.get(5)?,
                last_active_at: r.get(6)?,
                expires_at: r.get(7)?,
                owner_backend_pubkey: r.get(8)?,
                owner_host: r.get(9)?,
            })
        })?;
        mapped
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting legacy session_claims rows")?
    };

    conn.execute_batch(
        r#"
        DROP INDEX IF EXISTS idx_session_claims_expires;
        DROP INDEX IF EXISTS idx_session_claims_session;
        DROP TABLE session_claims;
        CREATE TABLE session_claims (
            pubkey TEXT NOT NULL,
            agent_slug TEXT NOT NULL DEFAULT '',
            codename TEXT NOT NULL DEFAULT '',
            session_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL DEFAULT '',
            native_id TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '',
            last_active_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            owner_backend_pubkey TEXT NOT NULL DEFAULT '',
            owner_host TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX IF NOT EXISTS idx_session_claims_expires ON session_claims(expires_at);
        CREATE INDEX IF NOT EXISTS idx_session_claims_session ON session_claims(session_id);
        "#,
    )
    .context("rebuilding session_claims table for per-session codename schema")?;

    for row in rows {
        let codename = crate::util::friendly_short_code(&row.session_id);
        conn.execute(
            "INSERT OR REPLACE INTO session_claims
                 (pubkey, agent_slug, codename, session_id, channel_h, native_id, harness,
                  last_active_at, expires_at, owner_backend_pubkey, owner_host)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                row.pubkey,
                row.agent_slug,
                codename,
                row.session_id,
                row.channel_h,
                row.native_id,
                row.harness,
                row.last_active_at,
                row.expires_at,
                row.owner_backend_pubkey,
                row.owner_host,
            ],
        )
        .context("backfilling migrated session_claims row")?;
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let cols = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reshapes_legacy_claims_and_backfills_codename() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_claims (
                pubkey TEXT NOT NULL,
                base_pubkey TEXT NOT NULL,
                agent_slug TEXT NOT NULL DEFAULT '',
                ordinal INTEGER NOT NULL DEFAULT 0,
                session_id TEXT NOT NULL DEFAULT '',
                channel_h TEXT NOT NULL DEFAULT '',
                native_id TEXT NOT NULL DEFAULT '',
                harness TEXT NOT NULL DEFAULT '',
                last_active_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (pubkey, channel_h)
            )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_claims
                 (pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h, native_id,
                  harness, last_active_at, expires_at)
             VALUES ('pk', 'base', 'smith', 2, 'sess-1', '#a', 'nat', 'claude-code', 10, 99)",
            [],
        )
        .unwrap();

        ensure_columns(&conn).unwrap();

        let cols = table_columns(&conn, "session_claims").unwrap();
        assert!(cols.contains("codename"), "codename column missing");
        assert!(!cols.contains("base_pubkey"), "base_pubkey should be gone");
        assert!(!cols.contains("ordinal"), "ordinal should be gone");
        assert!(cols.contains("owner_host"), "owner_host missing");

        let (codename, expires): (String, i64) = conn
            .query_row(
                "SELECT codename, expires_at FROM session_claims WHERE pubkey='pk'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(codename, crate::util::friendly_short_code("sess-1"));
        assert_eq!(expires, 99);
    }

    #[test]
    fn adds_owner_columns_to_new_shape_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session_claims (
                pubkey TEXT NOT NULL,
                agent_slug TEXT NOT NULL DEFAULT '',
                codename TEXT NOT NULL DEFAULT '',
                session_id TEXT NOT NULL DEFAULT '',
                channel_h TEXT NOT NULL DEFAULT '',
                native_id TEXT NOT NULL DEFAULT '',
                harness TEXT NOT NULL DEFAULT '',
                last_active_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (pubkey, channel_h)
            )",
        )
        .unwrap();

        ensure_columns(&conn).unwrap();

        let cols = table_columns(&conn, "session_claims").unwrap();
        for (name, _) in OWNER_COLUMNS {
            assert!(cols.contains(*name), "missing {name}");
        }
    }
}
