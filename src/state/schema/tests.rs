use crate::state::Store;
use rusqlite::Connection;

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
fn fresh_file_db_uses_only_canonical_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    let store = Store::open(&path).expect("fresh db opens");
    drop(store);

    let conn = Connection::open(&path).unwrap();
    assert!(table_exists(&conn, "workspace_roots"));
    assert!(table_exists(&conn, "trellis_replay_capsules"));
    assert!(table_exists(&conn, "durable_agent_sessions"));
    assert!(!table_exists(&conn, "project_roots"));

    let identities = columns(&conn, "identities");
    assert!(identities.iter().any(|c| c == "codename"));
    assert!(!identities.iter().any(|c| c == "base_pubkey"));
    assert!(!identities.iter().any(|c| c == "ordinal"));

    let claims = columns(&conn, "session_claims");
    assert!(claims.iter().any(|c| c == "owner_backend_pubkey"));
    assert!(claims.iter().any(|c| c == "owner_host"));
    assert!(!claims.iter().any(|c| c == "base_pubkey"));
    assert!(!claims.iter().any(|c| c == "ordinal"));

    assert!(columns(&conn, "relay_profiles")
        .iter()
        .any(|c| c == "agent_slug"));
    assert!(columns(&conn, "outbox")
        .iter()
        .any(|c| c == "next_attempt_at"));
    let sess_cols = columns(&conn, "sessions");
    assert!(
        sess_cols.iter().any(|c| c == "distill_notice_at"),
        "sessions.distill_notice_at"
    );
    assert!(
        sess_cols.iter().any(|c| c == "distill_fail_streak"),
        "sessions.distill_fail_streak"
    );
    assert!(
        sess_cols.iter().any(|c| c == "explicit_chat_published_at"),
        "sessions.explicit_chat_published_at"
    );
    assert!(
        sess_cols.iter().any(|c| c == "work_topic"),
        "sessions.work_topic"
    );
    assert!(
        sess_cols.iter().any(|c| c == "work_topic_set_at"),
        "sessions.work_topic_set_at"
    );
}

#[test]
fn schema_v1_is_rejected_instead_of_upgraded_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("DROP TABLE durable_agent_sessions", [])
            .unwrap();
    }

    let error = match Store::open(&path) {
        Ok(_) => panic!("schema v1 must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("schema version 1 is incompatible"));
    let conn = Connection::open(&path).unwrap();
    assert!(!table_exists(&conn, "durable_agent_sessions"));
}

#[test]
fn stamped_non_canonical_file_db_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE identities (
            pubkey TEXT NOT NULL,
            base_pubkey TEXT NOT NULL,
            ordinal INTEGER NOT NULL DEFAULT 0,
            session_id TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (base_pubkey, ordinal)
        );
        CREATE TABLE project_roots (
            channel_h TEXT PRIMARY KEY,
            abs_path TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        "#,
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 2u32).unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("non-canonical schema must be rejected"),
        Err(err) => err,
    };
    let text = format!("{err:#}");
    assert!(text.contains("not the current canonical schema"), "{text}");
}

#[test]
fn unstamped_existing_file_db_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE anything (id INTEGER)", [])
        .unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("unstamped db must be rejected"),
        Err(err) => err,
    };
    let text = format!("{err:#}");
    assert!(text.contains("has no schema version stamp"), "{text}");
}
