use super::*;

#[test]
fn schema_twenty_seven_gives_pending_event_claims_their_first_retry_time() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        ALTER TABLE event_claims DROP COLUMN attempts;
        ALTER TABLE event_claims DROP COLUMN retry_at;
        INSERT INTO event_claims (event_id, claim_key, state, created_at, updated_at)
        VALUES ('failed', 'orchestration:a', 'pending', 10, 100),
               ('done', 'orchestration:b', 'completed', 10, 100);
        PRAGMA user_version = 27;
        "#,
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twenty-seven upgrades to current"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 28);
    let row = |event_id: &str| {
        conn.query_row(
            "SELECT attempts, retry_at FROM event_claims WHERE event_id=?1",
            [event_id],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?)),
        )
        .unwrap()
    };
    assert_eq!(row("failed"), (1, 130));
    assert_eq!(row("done"), (0, 0));
}

#[test]
fn schema_twenty_seven_with_retry_columns_present_is_only_restamped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        INSERT INTO event_claims
            (event_id, claim_key, state, created_at, updated_at, attempts, retry_at)
        VALUES ('failed', 'orchestration:a', 'pending', 10, 100, 4, 900);
        PRAGMA user_version = 27;
        "#,
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("re-stamped schema twenty-seven opens"));
    let conn = Connection::open(&path).unwrap();
    assert_eq!(version(&conn), 28);
    assert_eq!(
        conn.query_row(
            "SELECT attempts, retry_at FROM event_claims WHERE event_id='failed'",
            [],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?)),
        )
        .unwrap(),
        (4, 900)
    );
}

#[test]
fn schema_twenty_six_migrates_through_both_steps_to_current() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        DROP TABLE profile_retirements;
        ALTER TABLE event_claims DROP COLUMN attempts;
        ALTER TABLE event_claims DROP COLUMN retry_at;
        INSERT INTO event_claims (event_id, claim_key, state, created_at, updated_at)
        VALUES ('failed', 'orchestration:a', 'pending', 10, 0);
        PRAGMA user_version = 26;
        "#,
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
    assert_eq!(
        conn.query_row(
            "SELECT attempts, retry_at FROM event_claims WHERE event_id='failed'",
            [],
            |row| Ok((row.get::<_, u32>(0)?, row.get::<_, u64>(1)?)),
        )
        .unwrap(),
        (1, 30)
    );
}
