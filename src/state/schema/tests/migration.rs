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
    assert_eq!(version(&conn), 9);
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
fn schema_eight_transport_backfill_is_harness_scoped_and_defaults_are_canonical() {
    let directory = tempfile::tempdir().unwrap();
    let migrated_path = directory.path().join("migrated.db");
    fixture::create_schema_eight(&migrated_path);

    drop(Store::open(&migrated_path).expect("schema eight upgrades to current"));

    let migrated = Connection::open(&migrated_path).unwrap();
    assert_eq!(version(&migrated), 9);
    assert_eq!(
        session_runtime_facts(&migrated, "pk-pty"),
        ("pty".to_string(), "migration".to_string())
    );
    assert_eq!(
        session_runtime_facts(&migrated, "pk-acp"),
        ("acp".to_string(), "migration".to_string())
    );
    assert_eq!(
        session_runtime_facts(&migrated, "pk-app-server"),
        ("app-server".to_string(), "migration".to_string())
    );
    assert_eq!(
        migrated
            .query_row(
                "SELECT locator_kind FROM session_locators WHERE pubkey='pk-app-server'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "app_server"
    );

    let fresh_path = directory.path().join("fresh.db");
    drop(Store::open(&fresh_path).expect("fresh schema opens"));
    let fresh = Connection::open(&fresh_path).unwrap();
    assert_eq!(
        column_default(&migrated, "sessions", "endpoint_provenance"),
        column_default(&fresh, "sessions", "endpoint_provenance")
    );
    assert_eq!(
        column_default(&migrated, "sessions", "endpoint_provenance").as_deref(),
        Some("''")
    );
}

#[test]
fn migration_chain_covers_every_version_before_current() {
    assert_eq!(
        super::super::migration::supported_versions(),
        [4, 5, 6, 7, 8]
    );
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

fn session_runtime_facts(conn: &Connection, pubkey: &str) -> (String, String) {
    conn.query_row(
        "SELECT admitted_transport, endpoint_provenance FROM sessions WHERE pubkey=?1",
        [pubkey],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .unwrap()
}

fn column_default(conn: &Connection, table: &str, column: &str) -> Option<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, Option<String>>(4)?))
        })
        .unwrap()
        .find_map(|row| {
            let (name, default) = row.unwrap();
            (name == column).then_some(default)
        })
        .flatten()
}
