use super::*;

fn downgraded_without(table: &str, version: u32) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let store = Store::open(&path).unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", version).unwrap();
    conn.execute(&format!("DROP TABLE {table}"), []).unwrap();
    (dir, path)
}

fn rejected(path: &std::path::Path, message: &str) -> anyhow::Error {
    match Store::open(path) {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

#[test]
fn schema_v1_is_rejected_instead_of_upgraded_in_place() {
    let (_dir, path) = downgraded_without("session_locators", 1);
    let error = rejected(&path, "schema v1 must be rejected");
    assert!(error
        .to_string()
        .contains("schema version 1 predates automatic migrations"));
    assert!(!table_exists(
        &Connection::open(&path).unwrap(),
        "session_locators"
    ));
}

#[test]
fn schema_v2_is_rejected_instead_of_preserving_session_id_derived_signers() {
    let (_dir, path) = downgraded_without("session_signers", 2);
    let error = rejected(&path, "schema v2 must be rejected");
    assert!(error
        .to_string()
        .contains("schema version 2 predates automatic migrations"));
    assert!(!table_exists(
        &Connection::open(&path).unwrap(),
        "session_signers"
    ));
}

#[test]
fn schema_v3_is_rejected_instead_of_preserving_session_keyed_inbox() {
    let (_dir, path) = downgraded_without("event_claims", 3);
    let error = rejected(&path, "schema v3 must be rejected");
    assert!(error
        .to_string()
        .contains("schema version 3 predates automatic migrations"));
    assert!(!table_exists(
        &Connection::open(&path).unwrap(),
        "event_claims"
    ));
}

#[test]
fn stamped_non_canonical_file_db_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        r#"
        CREATE TABLE identities (
            pubkey TEXT NOT NULL,
            base_pubkey TEXT NOT NULL,
            ordinal INTEGER NOT NULL DEFAULT 0,
            session_id TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (base_pubkey, ordinal)
        );
        CREATE TABLE project_roots (
            channel_h TEXT PRIMARY KEY,
            abs_path TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE unexpected_table (id INTEGER PRIMARY KEY);
        "#,
    )
    .unwrap();
    conn.pragma_update(None, "user_version", 16u32).unwrap();
    drop(conn);
    let err = rejected(&path, "non-canonical schema must be rejected");
    let text = format!("{err:#}");
    assert!(
        text.contains("schema 17 is missing table `sessions`"),
        "{text}"
    );
}

#[test]
fn unstamped_existing_file_db_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE anything (id INTEGER)", [])
        .unwrap();
    drop(conn);
    let err = rejected(&path, "unstamped db must be rejected");
    let text = format!("{err:#}");
    assert!(text.contains("has no schema version stamp"), "{text}");
}
