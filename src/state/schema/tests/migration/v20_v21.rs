use super::*;

#[test]
fn schema_twenty_adds_empty_session_coaching_ledger() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    conn.execute_batch(
        "DROP TABLE session_coaching;
         PRAGMA user_version = 20;",
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twenty upgrades to current"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 23);
    assert_eq!(
        crate::state::schema::tests::columns(&conn, "session_coaching"),
        ["pubkey", "runtime_generation", "code", "shown_at"]
    );
}

#[test]
fn malformed_schema_twenty_fails_without_mutating_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));
    let conn = Connection::open(&path).unwrap();
    fixture::restore_relay_derived_tables(&conn);
    conn.execute_batch(
        "DROP TABLE session_coaching;
         ALTER TABLE sessions DROP COLUMN runtime_generation;
         PRAGMA user_version = 20;",
    )
    .unwrap();
    drop(conn);

    let error = Store::open(&path)
        .err()
        .expect("malformed schema twenty must fail");
    assert!(format!("{error:#}").contains("missing `runtime_generation`"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 20);
    assert!(!crate::state::schema::tests::table_exists(
        &conn,
        "session_coaching"
    ));
}
