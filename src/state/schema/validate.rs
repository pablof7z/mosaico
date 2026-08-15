//! Fail-closed validation of the one current persistence shape.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::path::Path;

const TABLES: &[&str] = &[
    "channel_readiness_attempts",
    "channel_resolution_intents",
    "event_claims",
    "handle_leases",
    "inbox",
    "message_attachments",
    "native_turn_attempts",
    "mcp_actor_aliases",
    "nmp_event_arrivals",
    "receipts",
    "session_channels",
    "session_coaching",
    "session_locators",
    "session_signers",
    "session_standing",
    "sessions",
    "workspace_roots",
];
pub(super) fn canonical(conn: &Connection, path: Option<&Path>) -> Result<()> {
    ensure_only_tables(conn, path)?;
    for table in [
        "workspace_roots",
        "session_signers",
        "mcp_actor_aliases",
        "session_locators",
        "session_coaching",
        "event_claims",
        "native_turn_attempts",
        "nmp_event_arrivals",
        "message_attachments",
    ] {
        ensure_table(conn, table, path)?;
    }
    for table in [
        "project_roots",
        "session_aliases",
        "identities",
        "durable_agent_sessions",
        "session_claims",
        "llm_calls",
        "relay_agent_roster",
        "relay_channel_member_sets",
        "relay_channel_members",
        "relay_channels",
        "relay_events",
        "relay_profiles",
        "relay_reactions",
        "relay_status",
        "relay_status_sets",
        "messages",
        "message_recipients",
    ] {
        ensure_absent_table(conn, table, path)?;
    }
    validate_identity_and_delivery(conn, path)?;
    validate_session(conn, path)
}

fn validate_identity_and_delivery(conn: &Connection, path: Option<&Path>) -> Result<()> {
    ensure_columns(
        conn,
        "session_signers",
        &["pubkey", "signer_salt"],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "session_coaching",
        &["pubkey", "runtime_generation", "code", "shown_at"],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "session_locators",
        &[
            "harness",
            "locator_kind",
            "locator_value",
            "pubkey",
            "runtime_generation",
            "created_at",
        ],
        &["external_id_kind", "external_id", "session_id"],
        path,
    )?;
    ensure_columns(
        conn,
        "event_claims",
        &["event_id", "claim_key", "state", "updated_at"],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "native_turn_attempts",
        &[
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
        ],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "session_channels",
        &["pubkey", "channel_h", "joined_at", "joined_event_seq"],
        &["session_id", "granted_at"],
        path,
    )?;
    ensure_columns(
        conn,
        "session_standing",
        &[
            "pubkey",
            "channel_h",
            "state",
            "standing_epoch",
            "session_lifecycle_epoch",
        ],
        &["retain_until"],
        path,
    )?;
    ensure_columns(
        conn,
        "inbox",
        &["event_id", "target_pubkey", "state"],
        &["target_session"],
        path,
    )?;
    ensure_columns(
        conn,
        "nmp_event_arrivals",
        &["sequence", "event_id"],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "message_attachments",
        &["event_id", "directory"],
        &[],
        path,
    )
}

fn validate_session(conn: &Connection, path: Option<&Path>) -> Result<()> {
    ensure_columns(
        conn,
        "sessions",
        &[
            "pubkey",
            "runtime_generation",
            "work_root",
            "readiness_parent",
            "observed_harness",
            "claimed_harness",
            "admitted_preset",
            "admitted_transport",
            "endpoint_provenance",
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
            "busy_seconds",
            "state_changed_at",
        ],
        &[
            "session_id",
            "agent_pubkey",
            "resume_id",
            "last_distill_at",
            "distill_fail_streak",
            "distill_notice_at",
            "work_topic",
            "work_topic_set_at",
            "activity",
            "alive",
            "working",
            "harness",
            "explicit_chat_published_at",
            "transcript_path",
            "channel_h",
            "admitted_bundle",
        ],
        path,
    )
}

mod shape;
use shape::*;
