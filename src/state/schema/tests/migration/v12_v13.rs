use super::*;

#[test]
fn schema_twelve_backfills_semantic_state_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    fixture::downgrade_channel_context_to_v17(&conn);
    conn.execute("ALTER TABLE sessions DROP COLUMN state_changed_at", [])
        .unwrap();
    conn.execute("ALTER TABLE sessions DROP COLUMN busy_seconds", [])
        .unwrap();
    conn.execute("ALTER TABLE relay_status DROP COLUMN state_since", [])
        .unwrap();
    fixture::add_removed_v15_session_columns(&conn);
    conn.execute(
        "INSERT INTO relay_status
            (pubkey, channel_h, state, updated_at)
         VALUES ('peer', 'root', 'idle', 17)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions
            (pubkey, runtime_generation, agent_slug, work_state, created_at, turn_started_at)
         VALUES ('working', 1, 'codex', 'working', 10, 22)",
        [],
    )
    .unwrap();
    fixture::downgrade_messages_to_v19(&conn);
    conn.pragma_update(None, "user_version", 12).unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twelve upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 21);
    assert_eq!(
        conn.query_row(
            "SELECT state_since FROM relay_status WHERE pubkey='peer'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        17
    );
    assert_eq!(
        conn.query_row(
            "SELECT state_changed_at FROM sessions WHERE pubkey='working'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        22
    );
}
