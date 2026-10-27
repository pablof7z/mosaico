use super::*;

#[test]
fn schema_twenty_six_gains_durable_profile_retirements() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE profile_retirements;
         INSERT INTO session_notice VALUES ('pk1', 1, 'kept', 100);
         PRAGMA user_version = 26;",
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twenty-six upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 28);
    assert!(crate::state::schema::tests::table_exists(
        &conn,
        "profile_retirements"
    ));
    assert_eq!(count(&conn, "session_notice"), 1);
    conn.execute(
        "INSERT INTO profile_retirements (pubkey, due_at) VALUES ('pk1', 5)",
        [],
    )
    .unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT attempts FROM profile_retirements WHERE pubkey='pk1'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

#[test]
fn a_current_schema_without_retirement_attempts_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE profile_retirements;
         CREATE TABLE profile_retirements (
             pubkey TEXT PRIMARY KEY CHECK (pubkey <> ''),
             due_at INTEGER NOT NULL
         );",
    )
    .unwrap();
    drop(conn);

    assert!(Store::open(&path).is_err());
}
