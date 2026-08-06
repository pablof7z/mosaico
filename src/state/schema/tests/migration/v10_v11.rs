use super::*;

#[test]
fn schema_ten_consumes_only_idle_injected_rows() {
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
    fixture::downgrade_messages_to_v19(&conn);
    conn.pragma_update(None, "user_version", 10).unwrap();
    conn.execute_batch(
        r#"
        INSERT INTO sessions(pubkey, runtime_generation, agent_slug, work_state, created_at)
        VALUES ('idle', 1, 'grok', 'idle', 1),
               ('working', 1, 'grok', 'working', 1);
        INSERT INTO inbox(event_id, target_pubkey, state, created_at)
        VALUES ('idle-injected', 'idle', 'injected', 1),
               ('idle-pending', 'idle', 'pending', 1),
               ('working-injected', 'working', 'injected', 1);
        "#,
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema ten upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 21);
    let states = conn
        .prepare("SELECT event_id, state FROM inbox ORDER BY event_id")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        states,
        [
            ("idle-injected".into(), "echo_consumed".into()),
            ("idle-pending".into(), "pending".into()),
            ("working-injected".into(), "injected".into()),
        ]
    );
}
