use super::*;

#[test]
fn schema_fourteen_adds_zeroed_busy_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute("ALTER TABLE sessions DROP COLUMN busy_seconds", [])
        .unwrap();
    conn.execute(
        "INSERT INTO sessions
            (pubkey, runtime_generation, agent_slug, created_at, turn_count)
         VALUES ('historical', 1, 'codex', 10, 3)",
        [],
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 14).unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema fourteen upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 15);
    assert_eq!(
        conn.query_row(
            "SELECT busy_seconds FROM sessions WHERE pubkey='historical'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0
    );
}
