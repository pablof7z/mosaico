use super::*;

const REMOVED: [&str; 10] = [
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
];

#[test]
fn schema_twenty_one_extracts_local_satellites_and_drops_all_relay_state() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    conn.execute_batch(
        r#"
        INSERT INTO relay_channels
            (channel_h, name, created_at, updated_at)
        VALUES ('root', 'general', 1, 1);
        INSERT INTO relay_channel_members
            (channel_h, pubkey, role, updated_at)
        VALUES ('root', 'admin', 'admin', 1);
        INSERT INTO relay_channel_member_sets
            (channel_h, role, updated_at)
        VALUES ('root', 'admin', 1);
        INSERT INTO workspace_roots (channel_h, abs_path, updated_at)
        VALUES ('root', '/work/root', 2);
        INSERT INTO channel_resolution_intents
            (parent, name, channel_h, created_at)
        VALUES ('root', 'task', 'child', 3);
        INSERT INTO session_channels
            (pubkey, channel_h, joined_at, joined_event_seq)
        VALUES ('agent', 'root', 4, 0);
        INSERT INTO relay_events
            (id, kind, pubkey, created_at, channel_h, content)
        VALUES ('event', 9, 'author', 5, 'root', 'hello');
        INSERT INTO messages
            (message_id, channel_h, author_pubkey, created_at, attachment_dir)
        VALUES ('event', 'root', 'author', 5, '/downloads/event');
        PRAGMA user_version = 21;
        "#,
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).expect("schema twenty-one upgrades to current");
    assert_eq!(store.record_nmp_arrival("next-event").unwrap(), 2);
    drop(store);
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 22);
    for table in REMOVED {
        assert!(!fixture::table_exists(&conn, table), "{table} removed");
    }
    assert_eq!(count(&conn, "workspace_roots"), 1);
    assert_eq!(count(&conn, "channel_resolution_intents"), 1);
    assert_eq!(count(&conn, "session_channels"), 1);
    assert_eq!(
        conn.query_row(
            "SELECT event_id FROM nmp_event_arrivals WHERE sequence=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "event"
    );
    assert_eq!(
        conn.query_row(
            "SELECT event_id FROM nmp_event_arrivals WHERE sequence=2",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "next-event"
    );
    assert_eq!(
        conn.query_row(
            "SELECT directory FROM message_attachments WHERE event_id='event'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "/downloads/event"
    );
}

#[test]
fn noncanonical_relay_cache_shapes_are_destroyed_instead_of_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    conn.execute_batch(
        "ALTER TABLE relay_channels DROP COLUMN updated_at;
         PRAGMA user_version = 21;",
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("obsolete group cache shape is disposable"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 22);
    for table in REMOVED {
        assert!(!fixture::table_exists(&conn, table), "{table} removed");
    }
}
