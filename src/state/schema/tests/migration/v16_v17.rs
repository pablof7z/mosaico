use super::*;

#[test]
fn schema_sixteen_moves_backend_advertisements_into_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    fixture::downgrade_channel_context_to_v17(&conn);
    conn.execute("ALTER TABLE relay_profiles DROP COLUMN agents_json", [])
        .unwrap();
    conn.execute("ALTER TABLE relay_profiles DROP COLUMN workspaces_json", [])
        .unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE relay_agent_roster (
            backend_pubkey TEXT NOT NULL,
            agent_slug TEXT NOT NULL,
            channel_h TEXT NOT NULL,
            host TEXT NOT NULL DEFAULT '',
            use_criteria TEXT NOT NULL DEFAULT '',
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (backend_pubkey, agent_slug, channel_h)
        );
        INSERT INTO relay_profiles
            (pubkey, name, slug, host, is_backend, updated_at)
        VALUES ('backend', 'laptop', 'laptop', 'laptop', 1, 10);
        INSERT INTO relay_agent_roster
            (backend_pubkey, agent_slug, channel_h, host, use_criteria, updated_at)
        VALUES ('backend', 'codex', 'workspace', 'laptop', 'Writes code', 10);
        PRAGMA user_version = 16;
        "#,
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema sixteen upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 19);
    assert!(!fixture::table_exists(&conn, "relay_agent_roster"));
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM relay_profiles WHERE is_backend=1",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0,
        "stale backend profiles are refetched with complete host snapshots"
    );
    for column in ["agents_json", "workspaces_json"] {
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('relay_profiles')
                 WHERE name=?1",
                [column],
                |row| row.get::<_, u64>(0),
            )
            .unwrap(),
            1
        );
    }
}
