use crate::state::Store;
use rusqlite::Connection;

#[path = "tests/migration.rs"]
mod migration;
#[path = "tests/rejection.rs"]
mod rejection;
#[path = "tests/relay_state_authority.rs"]
mod relay_state_authority;
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
    assert_eq!(version, 24);
    assert!(table_exists(&conn, "workspace_roots"));
    assert!(table_exists(&conn, "session_locators"));
    assert!(!table_exists(&conn, "session_aliases"));
    assert!(!table_exists(&conn, "identities"));
    assert!(!table_exists(&conn, "durable_agent_sessions"));
    assert!(table_exists(&conn, "nmp_event_arrivals"));
    assert!(table_exists(&conn, "message_attachments"));
    for nmp_owned in [
        "relay_channels",
        "relay_channel_members",
        "relay_channel_member_sets",
        "relay_profiles",
        "relay_status",
        "relay_status_sets",
        "relay_events",
        "relay_reactions",
        "messages",
        "message_recipients",
    ] {
        assert!(
            !table_exists(&conn, nmp_owned),
            "{nmp_owned} must stay NMP-owned"
        );
    }
    assert!(!table_exists(&conn, "project_roots"));
    assert_eq!(
        columns(&conn, "native_turn_attempts"),
        [
            "id",
            "pubkey",
            "runtime_generation",
            "delivery_kind",
            "delivery_event_id",
            "native_thread_id",
            "native_turn_id",
            "outcome",
            "error_message",
            "error_details",
            "started_at",
            "finished_at",
        ]
    );

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
        ["pubkey", "channel_h", "joined_at", "joined_event_seq"]
    );
    assert_eq!(
        columns(&conn, "session_standing"),
        [
            "pubkey",
            "channel_h",
            "state",
            "standing_epoch",
            "session_lifecycle_epoch",
            "updated_at"
        ]
    );

    assert_eq!(
        columns(&conn, "nmp_event_arrivals"),
        ["sequence", "event_id"]
    );
    assert_eq!(
        columns(&conn, "message_attachments"),
        ["event_id", "directory"]
    );
    let sess_cols = columns(&conn, "sessions");
    assert!(sess_cols.iter().any(|c| c == "pubkey"));
    assert!(sess_cols.iter().any(|c| c == "runtime_generation"));
    assert!(sess_cols.iter().any(|c| c == "work_root"));
    assert!(sess_cols.iter().any(|c| c == "readiness_parent"));
    assert!(!sess_cols.iter().any(|c| c == "channel_h"));
    for admitted in [
        "observed_harness",
        "claimed_harness",
        "admitted_preset",
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
    for removed in [
        "last_distill_at",
        "distill_fail_streak",
        "distill_notice_at",
        "work_topic",
        "work_topic_set_at",
        "activity",
        "alive",
        "working",
        "explicit_chat_published_at",
        "transcript_path",
    ] {
        assert!(
            !sess_cols.iter().any(|c| c == removed),
            "sessions.{removed}"
        );
    }
}
