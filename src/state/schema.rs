//! The stamped persistence schema.
//! Six `relay_*` tables are materialized caches and may be dropped/rebuilt from
//! relay state. The remaining local tables are non-rebuildable daemon state:
//! runtime bindings and locators, inbox, event claims, channel
//! reservations, and workspace roots.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::path::Path;

mod ddl;
mod migration;
mod version;

use ddl::SCHEMA;
pub(crate) use migration::{load_pending_writes, replace_pending_writes};

const CANONICAL_TABLES: &[&str] = &[
    "channel_readiness_attempts",
    "channel_resolution_intents",
    "event_claims",
    "handle_leases",
    "inbox",
    "message_recipients",
    "messages",
    "mcp_actor_aliases",
    "receipts",
    "relay_agent_roster",
    "relay_channel_member_sets",
    "relay_channel_members",
    "relay_channels",
    "relay_event_quarantine",
    "relay_events",
    "relay_profiles",
    "relay_reactions",
    "relay_status",
    "session_channels",
    "session_locators",
    "session_signers",
    "session_standing",
    "sessions",
    "workspace_roots",
];

pub(super) fn initialize_file(conn: &mut Connection, path: &Path) -> Result<()> {
    let initial_version = version::read(conn)?;
    migration::upgrade(conn, path)?;
    if initial_version == version::SCHEMA_VERSION && has_user_tables(conn)? {
        validate_canonical(conn, Some(path))?;
    }
    conn.execute_batch(SCHEMA).context("creating schema")?;
    validate_canonical(conn, Some(path))?;
    version::stamp(conn)
}

pub(super) fn initialize_memory(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)
        .context("creating in-memory schema")?;
    validate_canonical(conn, None)?;
    version::stamp(conn)
}

fn validate_canonical(conn: &Connection, path: Option<&Path>) -> Result<()> {
    ensure_only_tables(conn, CANONICAL_TABLES, path)?;
    ensure_table(conn, "workspace_roots", path)?;
    ensure_absent_table(conn, "project_roots", path)?;
    ensure_table(conn, "session_signers", path)?;
    ensure_table(conn, "mcp_actor_aliases", path)?;
    ensure_table(conn, "session_locators", path)?;
    ensure_absent_table(conn, "session_aliases", path)?;
    ensure_absent_table(conn, "identities", path)?;
    ensure_absent_table(conn, "durable_agent_sessions", path)?;
    ensure_columns(
        conn,
        "session_signers",
        &["pubkey", "signer_salt"],
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
    ensure_absent_table(conn, "session_claims", path)?;
    ensure_columns(conn, "relay_profiles", &["agent_slug"], &[], path)?;
    ensure_columns(
        conn,
        "relay_status",
        &["state", "state_since"],
        &["busy"],
        path,
    )?;
    ensure_table(conn, "event_claims", path)?;
    ensure_columns(
        conn,
        "event_claims",
        &["event_id", "claim_key", "state", "updated_at"],
        &[],
        path,
    )?;
    ensure_columns(
        conn,
        "relay_status",
        &["pubkey", "channel_h"],
        &["session_id"],
        path,
    )?;
    ensure_columns(
        conn,
        "session_channels",
        &["pubkey", "channel_h", "granted_at"],
        &["session_id", "joined_at"],
        path,
    )?;
    ensure_columns(
        conn,
        "session_standing",
        &[
            "pubkey",
            "channel_h",
            "state",
            "retain_until",
            "standing_epoch",
            "session_lifecycle_epoch",
        ],
        &[],
        path,
    )?;
    ensure_absent_table(conn, "llm_calls", path)?;
    ensure_columns(
        conn,
        "inbox",
        &["event_id", "target_pubkey", "state"],
        &["target_session"],
        path,
    )?;
    ensure_columns(
        conn,
        "messages",
        &["message_id", "author_pubkey"],
        &["author_session"],
        path,
    )?;
    ensure_columns(
        conn,
        "message_recipients",
        &["message_id", "recipient_pubkey"],
        &["target_session"],
        path,
    )?;
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
            "admitted_bundle",
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
            "explicit_chat_published_at",
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
        ],
        path,
    )?;
    ensure_columns(
        conn,
        "messages",
        &["author_pubkey"],
        &["author_session"],
        path,
    )?;
    ensure_columns(
        conn,
        "message_recipients",
        &["recipient_pubkey"],
        &["target_session"],
        path,
    )?;
    Ok(())
}

fn ensure_only_tables(conn: &Connection, canonical: &[&str], path: Option<&Path>) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master \
         WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )?;
    let actual = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    let expected = canonical.iter().copied().map(str::to_string).collect();
    if actual == expected {
        return Ok(());
    }
    let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
    let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
    bail_non_canonical(
        path,
        format!("table set differs; unexpected={unexpected:?}, missing={missing:?}"),
    )
}

fn ensure_table(conn: &Connection, table: &str, path: Option<&Path>) -> Result<()> {
    if table_exists(conn, table)? {
        return Ok(());
    }
    bail_non_canonical(path, format!("missing table `{table}`"))
}

fn ensure_absent_table(conn: &Connection, table: &str, path: Option<&Path>) -> Result<()> {
    if !table_exists(conn, table)? {
        return Ok(());
    }
    bail_non_canonical(path, format!("removed table `{table}` is still present"))
}

fn ensure_columns(
    conn: &Connection,
    table: &str,
    required: &[&str],
    forbidden: &[&str],
    path: Option<&Path>,
) -> Result<()> {
    let columns = table_columns(conn, table)?;
    for column in required {
        if !columns.contains(*column) {
            return bail_non_canonical(path, format!("`{table}` missing column `{column}`"));
        }
    }
    for column in forbidden {
        if columns.contains(*column) {
            return bail_non_canonical(path, format!("`{table}` still has column `{column}`"));
        }
    }
    Ok(())
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .with_context(|| format!("checking for table `{table}`"))
}

fn has_user_tables(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM sqlite_master
            WHERE type='table' AND name NOT LIKE 'sqlite_%'
        )",
        [],
        |row| row.get(0),
    )
    .context("checking for existing schema tables")
}

fn table_columns(conn: &Connection, table: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .with_context(|| format!("reading `{table}` columns"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()
        .with_context(|| format!("collecting `{table}` columns"))?;
    Ok(columns)
}

fn bail_non_canonical<T>(path: Option<&Path>, reason: String) -> Result<T> {
    match path {
        Some(path) => anyhow::bail!(
            "refusing to open {}: state.db is not the current canonical schema ({reason})",
            path.display()
        ),
        None => {
            anyhow::bail!("in-memory state schema is not the current canonical schema ({reason})")
        }
    }
}

#[cfg(test)]
mod tests;
