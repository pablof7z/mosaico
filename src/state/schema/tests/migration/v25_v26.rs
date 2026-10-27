use super::*;

#[test]
fn schema_twenty_five_renames_session_coaching_to_session_notice_preserving_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE session_coaching (
            pubkey             TEXT NOT NULL,
            runtime_generation INTEGER NOT NULL CHECK (runtime_generation > 0),
            code               TEXT NOT NULL CHECK (code <> ''),
            shown_at           INTEGER NOT NULL,
            PRIMARY KEY (pubkey, runtime_generation, code)
        );
        INSERT INTO session_coaching VALUES ('pk1', 1, 'unhosted_return_path', 100);
        DROP TABLE session_notice;
        PRAGMA user_version = 25;
        "#,
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twenty-five upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 28);
    assert!(crate::state::schema::tests::table_exists(
        &conn,
        "session_notice"
    ));
    assert!(!crate::state::schema::tests::table_exists(
        &conn,
        "session_coaching"
    ));
    assert_eq!(
        conn.query_row(
            "SELECT pubkey || ':' || code FROM session_notice WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap(),
        "pk1:unhosted_return_path"
    );
}
