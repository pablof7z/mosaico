use super::*;

#[test]
fn schema_eighteen_upgrades_then_drops_relay_owned_messages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    conn.execute(
        "INSERT INTO messages
            (message_id, channel_h, author_pubkey, body, created_at)
         VALUES ('event', 'room', 'sender', 'hello', 1)",
        [],
    )
    .unwrap();
    fixture::downgrade_messages_to_v19(&conn);
    conn.execute_batch(
        "ALTER TABLE messages DROP COLUMN attachment_dir;
         PRAGMA user_version = 18;",
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema eighteen upgrades to current"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 23);
    assert!(!fixture::table_exists(&conn, "messages"));
    assert_eq!(count(&conn, "message_attachments"), 0);
}

#[test]
fn malformed_schema_eighteen_fails_without_mutating_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    fixture::downgrade_messages_to_v19(&conn);
    conn.execute_batch(
        "ALTER TABLE messages DROP COLUMN attachment_dir;
         ALTER TABLE messages DROP COLUMN body;
         PRAGMA user_version = 18;",
    )
    .unwrap();
    drop(conn);

    let error = Store::open(&path)
        .err()
        .expect("partial schema eighteen must fail");
    assert!(format!("{error:#}").contains("missing `body`"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 18);
    assert!(!crate::state::schema::tests::columns(&conn, "messages")
        .contains(&"attachment_dir".to_string()));
}
