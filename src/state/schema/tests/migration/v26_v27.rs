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
    assert_eq!(version(&conn), 27);
    assert!(crate::state::schema::tests::table_exists(
        &conn,
        "profile_retirements"
    ));
    assert_eq!(count(&conn, "session_notice"), 1);
}
