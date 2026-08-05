use super::*;

#[test]
fn schema_thirteen_adds_native_turn_attempts_forward_only() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    fixture::downgrade_channel_context_to_v17(&conn);
    conn.execute("DROP TABLE native_turn_attempts", []).unwrap();
    conn.execute("ALTER TABLE sessions DROP COLUMN busy_seconds", [])
        .unwrap();
    fixture::add_removed_v15_session_columns(&conn);
    fixture::downgrade_messages_to_v19(&conn);
    conn.pragma_update(None, "user_version", 13).unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema thirteen upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 20);
    assert!(fixture::table_exists(&conn, "native_turn_attempts"));
}
