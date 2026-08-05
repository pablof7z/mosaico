use super::*;

#[test]
fn schema_seventeen_migrates_single_channel_pointer_without_losing_existing_memberships() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    fixture::downgrade_channel_context_to_v17(&conn);
    fixture::downgrade_messages_to_v19(&conn);
    conn.execute_batch(
        r#"
        INSERT INTO relay_events
            (id, kind, pubkey, created_at, channel_h)
        VALUES ('old-1', 9, 'human', 5, 'launch'),
               ('old-2', 9, 'human', 6, 'passive');
        INSERT INTO sessions
            (pubkey, runtime_generation, agent_slug, channel_h, work_root, created_at)
        VALUES ('with-route', 1, 'codex', 'launch', 'workspace', 10),
               ('pointer-only', 1, 'codex', 'orphaned', 'workspace', 12);
        INSERT INTO session_channels
            (pubkey, channel_h, granted_at)
        VALUES ('with-route', 'launch', 7),
               ('with-route', 'passive', 9);
        INSERT INTO session_standing
            (pubkey, channel_h, state, retain_until, standing_epoch,
             session_lifecycle_epoch, updated_at)
        VALUES ('with-route', 'launch', 'retained', 99, 2, 1, 11),
               ('pointer-only', 'cleanup', 'retained', 99, 3, 1, 12);
        INSERT INTO relay_channels
            (channel_h, name, parent, created_at, updated_at)
        VALUES ('existing-root', 'general', '', 1, 1),
               ('existing-child', 'named', 'existing-root', 1, 1);
        INSERT INTO relay_status
            (pubkey, channel_h, state)
        VALUES ('peer', 'launch', 'idle');
        PRAGMA user_version = 17;
        "#,
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).expect("schema seventeen upgrades to current");
    let cleanup_due = store.list_cleanup_due_member_standing().unwrap();
    assert_eq!(cleanup_due.len(), 1);
    assert_eq!(cleanup_due[0].pubkey, "pointer-only");
    assert_eq!(cleanup_due[0].channel_h, "cleanup");
    drop(store);
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 20);
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
              WHERE type='table' AND name='relay_event_quarantine'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0
    );
    assert!(!columns(&conn, "sessions")
        .iter()
        .any(|column| column == "channel_h"));
    assert_eq!(
        columns(&conn, "session_standing"),
        [
            "pubkey",
            "channel_h",
            "state",
            "standing_epoch",
            "session_lifecycle_epoch",
            "updated_at",
        ]
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM session_standing
              WHERE pubkey='with-route' AND channel_h='launch'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "member"
    );
    assert_eq!(
        conn.prepare(
            "SELECT channel_h, joined_at, joined_event_seq
               FROM session_channels
              ORDER BY pubkey, channel_h"
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap(),
        [
            ("orphaned".into(), 12, 2),
            ("launch".into(), 7, 2),
            ("passive".into(), 9, 2),
        ]
    );
    assert_eq!(
        conn.query_row(
            "SELECT workspace || ':' || branch FROM relay_status WHERE pubkey='peer'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        ":"
    );
    assert_eq!(
        conn.query_row(
            "SELECT updated_at FROM relay_status_sets WHERE pubkey='peer'",
            [],
            |row| row.get::<_, u64>(0),
        )
        .unwrap(),
        0
    );

    conn.execute(
        "INSERT INTO relay_channels
             (channel_h, name, parent, created_at, updated_at)
         VALUES ('other-root', 'general', '', 2, 2)",
        [],
    )
    .unwrap();
    for channel_h in ["unnamed-a", "unnamed-b"] {
        conn.execute(
            "INSERT INTO relay_channels
                 (channel_h, name, parent, created_at, updated_at)
             VALUES (?1, '', 'existing-root', 2, 2)",
            [channel_h],
        )
        .unwrap();
    }
    assert!(conn
        .execute(
            "INSERT INTO relay_channels
                 (channel_h, name, parent, created_at, updated_at)
             VALUES ('duplicate-child', 'named', 'existing-root', 2, 2)",
            [],
        )
        .is_err());
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
