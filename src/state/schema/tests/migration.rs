use rusqlite::Connection;

use crate::state::Store;

#[path = "migration_fixture.rs"]
mod fixture;

#[test]
fn deployed_schema_four_migrates_to_current_without_losing_local_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    fixture::create_schema_four(&path);

    drop(Store::open(&path).expect("schema four upgrades to current"));

    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 8);
    assert_eq!(
        conn.query_row("SELECT title FROM sessions WHERE pubkey='pk1'", [], |row| {
            row.get::<_, String>(0)
        },)
            .unwrap(),
        "newest"
    );
    assert_eq!(
        conn.query_row(
            "SELECT work_root || ':' || readiness_parent FROM sessions WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        ":"
    );
    assert_eq!(
        conn.query_row(
            "SELECT locator_kind || ':' || locator_value FROM session_locators WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "native_resume:resume-new"
    );
    assert_eq!(count(&conn, "messages"), 1);
    assert_eq!(count(&conn, "message_recipients"), 1);
    assert_eq!(
        conn.query_row("SELECT delivered_at FROM message_recipients", [], |row| row
            .get::<_, i64>(0),)
            .unwrap(),
        20
    );
    for table in ["sessions", "session_signers", "inbox", "workspace_roots"] {
        assert_eq!(count(&conn, table), 1, "{table} data survives");
    }
    for removed in [
        "outbox",
        "trellis_commits",
        "trellis_replay_capsules",
        "llm_calls",
    ] {
        assert!(!fixture::table_exists(&conn, removed), "{removed} removed");
    }
    assert_eq!(
        crate::state::load_pending_writes(&path).unwrap(),
        vec![r#"{"id":"pending"}"#]
    );
}

#[test]
fn malformed_schema_seven_fails_before_mutating_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    fixture::create_schema_seven(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute("DROP TABLE trellis_commits", []).unwrap();
    drop(conn);

    let error = Store::open(&path)
        .err()
        .expect("invalid schema seven fails");
    assert!(format!("{error:#}").contains("missing table `trellis_commits`"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 7);
    assert!(fixture::table_exists(&conn, "outbox"));
    assert!(crate::state::load_pending_writes(&path).unwrap().is_empty());
}

#[test]
fn migration_chain_covers_every_version_before_current() {
    assert_eq!(super::super::migration::supported_versions(), [4, 5, 6, 7]);
}

fn version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap()
}

fn count(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
