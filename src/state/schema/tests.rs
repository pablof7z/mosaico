//! Full-chain migration coverage: the per-table `ensure_*` migrations each have
//! isolated in-memory unit tests, but nothing exercises the ordered chain that
//! `Store::open` runs against a real, populated legacy FILE db. This drives that
//! path end to end — a hand-built legacy `state.db` opened through the real
//! `Store::open` — and asserts every migration applied and every row survived.

use crate::state::Store;
use rusqlite::Connection;

/// Stand up a legacy on-disk `state.db`: the pre-per-session schema for
/// `identities` and `session_claims` (both carrying `base_pubkey` + `ordinal`),
/// the old `project_roots` table, a `sessions` table without the distill
/// columns, a `relay_profiles` without `agent_slug`, and an `outbox` without
/// `next_attempt_at`. Stamped `user_version = 1` so the real open path treats it
/// as a versioned legacy db rather than refusing it.
fn write_legacy_db(path: &std::path::Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        r#"
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
            PRIMARY KEY (base_pubkey, ordinal)
        );
        CREATE INDEX idx_identities_base ON identities(base_pubkey);

        CREATE TABLE session_claims (
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
        );

        CREATE TABLE project_roots (
            channel_h TEXT PRIMARY KEY,
            abs_path TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );

        CREATE TABLE sessions (
            session_id        TEXT PRIMARY KEY,
            agent_pubkey      TEXT NOT NULL,
            agent_slug        TEXT NOT NULL DEFAULT '',
            channel_h         TEXT NOT NULL DEFAULT '',
            harness           TEXT NOT NULL DEFAULT '',
            child_pid         INTEGER,
            transcript_path   TEXT,
            alive             INTEGER NOT NULL DEFAULT 1,
            created_at        INTEGER NOT NULL,
            last_seen         INTEGER NOT NULL DEFAULT 0,
            working           INTEGER NOT NULL DEFAULT 0,
            turn_started_at   INTEGER NOT NULL DEFAULT 0,
            last_distill_at   INTEGER NOT NULL DEFAULT 0,
            seen_cursor       INTEGER NOT NULL DEFAULT 0,
            title             TEXT NOT NULL DEFAULT '',
            activity          TEXT NOT NULL DEFAULT '',
            resume_id         TEXT NOT NULL DEFAULT ''
        );

        CREATE TABLE relay_profiles (
            pubkey      TEXT PRIMARY KEY,
            name        TEXT NOT NULL DEFAULT '',
            slug        TEXT NOT NULL DEFAULT '',
            host        TEXT NOT NULL DEFAULT '',
            is_backend  INTEGER NOT NULL DEFAULT 0,
            updated_at  INTEGER NOT NULL
        );

        CREATE TABLE outbox (
            local_id     INTEGER PRIMARY KEY AUTOINCREMENT,
            event_json   TEXT NOT NULL,
            state        TEXT NOT NULL DEFAULT 'pending',
            retries      INTEGER NOT NULL DEFAULT 0,
            last_error   TEXT,
            enqueued_at  INTEGER NOT NULL
        );
        -- A pre-#295 db already carried this index (on the old column set); the
        -- canonical schema's `CREATE INDEX IF NOT EXISTS idx_outbox_pending`
        -- (which references the not-yet-added next_attempt_at) is skipped by name.
        CREATE INDEX idx_outbox_pending ON outbox(state, local_id);

        INSERT INTO identities
            (pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h, native_id, alive, created_at)
            VALUES ('id-pk', 'id-base', 'coder', 3, 'id-sess', '#room', 'id-native', 1, 5);
        INSERT INTO session_claims
            (pubkey, base_pubkey, agent_slug, ordinal, session_id, channel_h, native_id, harness, last_active_at, expires_at)
            VALUES ('cl-pk', 'cl-base', 'reviewer', 2, 'cl-sess', '#room', 'cl-native', 'claude-code', 11, 88);
        INSERT INTO project_roots (channel_h, abs_path, updated_at)
            VALUES ('#root', '/abs/project', 42);
        INSERT INTO sessions (session_id, agent_pubkey, created_at)
            VALUES ('sess-legacy', 'sess-pk', 9);
        INSERT INTO relay_profiles (pubkey, name, slug, host, is_backend, updated_at)
            VALUES ('prof-pk', 'display', 'coder', 'laptop', 0, 3);
        INSERT INTO outbox (event_json, enqueued_at) VALUES ('{}', 1);
        "#,
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 1u32).unwrap();
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn full_chain_migration_on_populated_legacy_file_db() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    write_legacy_db(&path);

    // The real open path runs the ordered migration chain.
    let store = Store::open(&path).expect("legacy db opens and migrates cleanly");
    drop(store);

    // Inspect the migrated file with a fresh reader.
    let conn = Connection::open(&path).unwrap();

    // identities: reshaped to per-session codename, rows preserved & backfilled.
    let id_cols = columns(&conn, "identities");
    assert!(
        id_cols.iter().any(|c| c == "codename"),
        "identities.codename"
    );
    assert!(
        !id_cols.iter().any(|c| c == "base_pubkey"),
        "identities.base_pubkey should be gone"
    );
    assert!(
        !id_cols.iter().any(|c| c == "ordinal"),
        "identities.ordinal should be gone"
    );
    let (id_codename, id_native): (String, String) = conn
        .query_row(
            "SELECT codename, native_id FROM identities WHERE pubkey='id-pk'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("legacy identity row preserved");
    assert_eq!(id_codename, crate::util::friendly_short_code("id-sess"));
    assert_eq!(id_native, "id-native");

    // session_claims: reshaped + owner columns added, row preserved.
    let cl_cols = columns(&conn, "session_claims");
    assert!(
        cl_cols.iter().any(|c| c == "codename"),
        "session_claims.codename"
    );
    assert!(
        cl_cols.iter().any(|c| c == "owner_host"),
        "session_claims.owner_host"
    );
    assert!(
        !cl_cols.iter().any(|c| c == "base_pubkey"),
        "session_claims.base_pubkey should be gone"
    );
    let (cl_codename, cl_expires): (String, i64) = conn
        .query_row(
            "SELECT codename, expires_at FROM session_claims WHERE pubkey='cl-pk'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("legacy session_claims row preserved");
    assert_eq!(cl_codename, crate::util::friendly_short_code("cl-sess"));
    assert_eq!(cl_expires, 88);

    // project_roots renamed to workspace_roots, rows carried over.
    assert!(
        !table_exists(&conn, "project_roots"),
        "project_roots must be gone"
    );
    assert!(
        table_exists(&conn, "workspace_roots"),
        "workspace_roots must exist"
    );
    let ws_path: String = conn
        .query_row(
            "SELECT abs_path FROM workspace_roots WHERE channel_h='#root'",
            [],
            |r| r.get(0),
        )
        .expect("workspace root row migrated");
    assert_eq!(ws_path, "/abs/project");

    // sessions gained the distill columns; the legacy row is intact.
    let sess_cols = columns(&conn, "sessions");
    assert!(
        sess_cols.iter().any(|c| c == "distill_notice_at"),
        "sessions.distill_notice_at"
    );
    assert!(
        sess_cols.iter().any(|c| c == "distill_fail_streak"),
        "sessions.distill_fail_streak"
    );
    let sess_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sessions WHERE session_id='sess-legacy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sess_count, 1, "legacy session row preserved");

    // relay_profiles gained agent_slug; outbox gained next_attempt_at.
    assert!(
        columns(&conn, "relay_profiles")
            .iter()
            .any(|c| c == "agent_slug"),
        "relay_profiles.agent_slug"
    );
    assert!(
        columns(&conn, "outbox")
            .iter()
            .any(|c| c == "next_attempt_at"),
        "outbox.next_attempt_at"
    );
}

#[test]
fn reopening_migrated_file_db_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    write_legacy_db(&path);

    // Open twice; the second pass must be a clean no-op over the already-migrated
    // file, and the migrated data must be unchanged.
    Store::open(&path).expect("first open migrates");
    Store::open(&path).expect("second open of the same file is safe");

    let conn = Connection::open(&path).unwrap();
    let id_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM identities", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id_count, 1, "no rows duplicated or lost on reopen");
    let codename: String = conn
        .query_row(
            "SELECT codename FROM identities WHERE pubkey='id-pk'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(codename, crate::util::friendly_short_code("id-sess"));
}
