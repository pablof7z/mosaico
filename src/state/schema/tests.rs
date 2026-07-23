use crate::state::Store;
use rusqlite::Connection;

#[path = "tests/migration.rs"]
mod migration;
#[path = "tests/session_context.rs"]
mod session_context;

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [name],
        |r| r.get(0),
    )
    .unwrap()
}

#[test]
fn fresh_file_db_uses_only_canonical_schema() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");

    let store = Store::open(&path).expect("fresh db opens");
    drop(store);

    let conn = Connection::open(&path).unwrap();
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 13);
    assert!(table_exists(&conn, "workspace_roots"));
    assert!(table_exists(&conn, "session_locators"));
    assert!(!table_exists(&conn, "session_aliases"));
    assert!(!table_exists(&conn, "identities"));
    assert!(!table_exists(&conn, "durable_agent_sessions"));
    assert!(table_exists(&conn, "relay_reactions"));
    assert!(!table_exists(&conn, "project_roots"));

    let reactions = columns(&conn, "relay_reactions");
    for col in [
        "reaction_id",
        "target_message_id",
        "channel_h",
        "reactor_pubkey",
        "emoji",
        "created_at",
    ] {
        assert!(reactions.iter().any(|c| c == col), "relay_reactions.{col}");
    }

    assert_eq!(
        columns(&conn, "session_locators"),
        [
            "harness",
            "locator_kind",
            "locator_value",
            "pubkey",
            "runtime_generation",
            "created_at"
        ]
    );

    assert_eq!(columns(&conn, "session_signers"), ["pubkey", "signer_salt"]);

    assert!(!table_exists(&conn, "session_claims"));
    assert_eq!(
        columns(&conn, "session_channels"),
        ["pubkey", "channel_h", "granted_at"]
    );
    assert_eq!(
        columns(&conn, "session_standing"),
        [
            "pubkey",
            "channel_h",
            "state",
            "retain_until",
            "standing_epoch",
            "session_lifecycle_epoch",
            "updated_at"
        ]
    );

    assert!(columns(&conn, "relay_profiles")
        .iter()
        .any(|c| c == "agent_slug"));
    let messages = columns(&conn, "messages");
    assert!(messages.iter().any(|c| c == "author_pubkey"));
    assert!(!messages.iter().any(|c| c == "author_session"));
    let recipients = columns(&conn, "message_recipients");
    assert!(recipients.iter().any(|c| c == "recipient_pubkey"));
    assert!(!recipients.iter().any(|c| c == "target_session"));
    let sess_cols = columns(&conn, "sessions");
    assert!(sess_cols.iter().any(|c| c == "pubkey"));
    assert!(sess_cols.iter().any(|c| c == "runtime_generation"));
    assert!(sess_cols.iter().any(|c| c == "work_root"));
    assert!(sess_cols.iter().any(|c| c == "readiness_parent"));
    for admitted in [
        "observed_harness",
        "claimed_harness",
        "admitted_bundle",
        "admitted_transport",
        "endpoint_provenance",
    ] {
        assert!(
            sess_cols.iter().any(|column| column == admitted),
            "sessions.{admitted}"
        );
    }
    for lifecycle in [
        "runtime_state",
        "presentation_state",
        "work_state",
        "recovery_state",
        "lifecycle_epoch",
        "attachment_epoch",
        "idle_since",
        "idle_deadline",
        "stopped_at",
        "stop_reason",
        "turn_count",
    ] {
        assert!(
            sess_cols.iter().any(|column| column == lifecycle),
            "{lifecycle}"
        );
    }
    assert!(!sess_cols.iter().any(|c| c == "harness"));
    assert!(!sess_cols.iter().any(|c| c == "session_id"));
    assert!(!sess_cols.iter().any(|c| c == "agent_pubkey"));
    assert!(!sess_cols.iter().any(|c| c == "resume_id"));
    assert!(!table_exists(&conn, "llm_calls"));
    assert!(
        sess_cols.iter().any(|c| c == "explicit_chat_published_at"),
        "sessions.explicit_chat_published_at"
    );
    for removed in [
        "last_distill_at",
        "distill_fail_streak",
        "distill_notice_at",
        "work_topic",
        "work_topic_set_at",
        "activity",
        "alive",
        "working",
    ] {
        assert!(
            !sess_cols.iter().any(|c| c == removed),
            "sessions.{removed}"
        );
    }
}

#[test]
fn schema_v1_is_rejected_instead_of_upgraded_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("DROP TABLE session_locators", []).unwrap();
    }

    let error = match Store::open(&path) {
        Ok(_) => panic!("schema v1 must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("schema version 1 predates automatic migrations"));
    let conn = Connection::open(&path).unwrap();
    assert!(!table_exists(&conn, "session_locators"));
}

#[test]
fn schema_v2_is_rejected_instead_of_preserving_session_id_derived_signers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute("DROP TABLE session_signers", []).unwrap();
    }

    let error = match Store::open(&path) {
        Ok(_) => panic!("schema v2 must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("schema version 2 predates automatic migrations"));
    let conn = Connection::open(&path).unwrap();
    assert!(!table_exists(&conn, "session_signers"));
}

#[test]
fn schema_v3_is_rejected_instead_of_preserving_session_keyed_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    {
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        conn.execute("DROP TABLE event_claims", []).unwrap();
    }

    let error = match Store::open(&path) {
        Ok(_) => panic!("schema v3 must be rejected"),
        Err(error) => error,
    };
    assert!(error
        .to_string()
        .contains("schema version 3 predates automatic migrations"));
    let conn = Connection::open(&path).unwrap();
    assert!(!table_exists(&conn, "event_claims"));
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
    conn.pragma_update(None, "user_version", 13u32).unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("non-canonical schema must be rejected"),
        Err(err) => err,
    };
    let text = format!("{err:#}");
    assert!(text.contains("not the current canonical schema"), "{text}");
}

#[test]
fn unstamped_existing_file_db_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute("CREATE TABLE anything (id INTEGER)", [])
        .unwrap();
    drop(conn);

    let err = match Store::open(&path) {
        Ok(_) => panic!("unstamped db must be rejected"),
        Err(err) => err,
    };
    let text = format!("{err:#}");
    assert!(text.contains("has no schema version stamp"), "{text}");
}
