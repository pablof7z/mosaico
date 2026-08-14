use crate::state::Store;
use rusqlite::Connection;
#[path = "migration_fixture.rs"]
mod fixture;
#[path = "migration/v10_v11.rs"]
mod v10_v11;
#[path = "migration/v12_v13.rs"]
mod v12_v13;
#[path = "migration/v13_v14.rs"]
mod v13_v14;
#[path = "migration/v14_v15.rs"]
mod v14_v15;
#[path = "migration/v15_v16.rs"]
mod v15_v16;
#[path = "migration/v16_v17.rs"]
mod v16_v17;
#[path = "migration/v17_v18.rs"]
mod v17_v18;
#[path = "migration/v18_v19.rs"]
mod v18_v19;
#[path = "migration/v20_v21.rs"]
mod v20_v21;
#[path = "migration/v21_v22.rs"]
mod v21_v22;
#[path = "migration/v22_v23.rs"]
mod v22_v23;
#[test]
fn deployed_schema_four_migrates_to_current_without_losing_local_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    fixture::create_schema_four(&path);
    drop(Store::open(&path).expect("schema four upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 23);
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
    assert_eq!(
        conn.query_row(
            "SELECT runtime_state || ':' || recovery_state || ':' || turn_count
             FROM sessions WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "running:ready:1"
    );
    assert!(
        conn.query_row(
            "SELECT state_changed_at FROM sessions WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap()
            > 0
    );
    assert_eq!(
        conn.query_row(
            "SELECT channel_h || ':' || joined_at FROM session_channels WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "room:8"
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM session_standing WHERE pubkey='pk1' AND channel_h='room'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "absent"
    );
    assert_eq!(count(&conn, "message_attachments"), 0);
    for table in ["sessions", "session_signers", "inbox", "workspace_roots"] {
        assert_eq!(count(&conn, table), 1, "{table} data survives");
    }
    for removed in [
        "outbox",
        "trellis_commits",
        "trellis_replay_capsules",
        "llm_calls",
        "session_claims",
        "relay_channels",
        "relay_channel_members",
        "relay_channel_member_sets",
        "relay_profiles",
        "relay_status",
        "relay_status_sets",
        "relay_events",
        "relay_reactions",
        "messages",
        "message_recipients",
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
    assert_eq!(version(&migrated), 23);
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
        [4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22,]
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
