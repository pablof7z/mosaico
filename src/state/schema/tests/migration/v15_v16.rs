use super::*;

#[test]
fn schema_fifteen_drops_the_auto_publish_marker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    fixture::downgrade_channel_context_to_v17(&conn);
    fixture::add_removed_v15_session_columns(&conn);
    conn.execute(
        "INSERT INTO sessions
            (pubkey, runtime_generation, agent_slug, created_at,
             explicit_chat_published_at, transcript_path)
         VALUES ('historical', 1, 'codex', 10, 12, '/private/transcript.jsonl')",
        [],
    )
    .unwrap();
    fixture::downgrade_messages_to_v19(&conn);
    conn.pragma_update(None, "user_version", 15).unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema fifteen upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 24);
    assert_eq!(
        conn.query_row(
            "SELECT agent_slug FROM sessions WHERE pubkey='historical'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "codex"
    );
    assert!(["explicit_chat_published_at", "transcript_path"]
        .iter()
        .all(|removed| !columns(&conn, "sessions")
            .iter()
            .any(|column| column == removed)));
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
