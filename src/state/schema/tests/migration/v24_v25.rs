use super::*;

#[test]
fn schema_twenty_four_renames_the_arrival_ledger_without_losing_its_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        ALTER TABLE fava_event_arrivals RENAME TO arrival_ledger_before_fava;
        INSERT INTO arrival_ledger_before_fava (event_id) VALUES ('first');
        PRAGMA user_version = 24;
        "#,
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).expect("schema twenty-four upgrades to current");
    assert_eq!(store.record_fava_arrival("second").unwrap(), 2);
    drop(store);

    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 25);
    assert!(table_exists(&conn, "fava_event_arrivals"));
    assert!(!table_exists(&conn, "arrival_ledger_before_fava"));
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}
