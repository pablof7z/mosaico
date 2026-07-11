pub(super) struct TableCoverage {
    pub(super) table: &'static str,
    pub(super) mode: &'static str,
    pub(super) targets: &'static str,
    pub(super) proves: &'static str,
}

pub(super) const DURABLE_TABLES: &[TableCoverage] = &[
    row(
        "channel_readiness_attempts",
        "direct",
        "readiness_attempt:<id> | provider_attempt:<id>",
        "provider readiness decisions",
    ),
    row(
        "channel_resolution_intents",
        "aggregate",
        "table:channel_resolution_intents | channel:<h>",
        "pending channel-name reservations",
    ),
    row(
        "handle_leases",
        "aggregate",
        "identity:<pubkey> | profile:<pubkey>",
        "authoritative current public handle ownership",
    ),
    row(
        "identities",
        "direct",
        "identity:<pubkey> | agent:<slug> | backend:<label>",
        "local identity/session bindings",
    ),
    row("inbox", "direct", "inbox:<event>", "inbound delivery state"),
    row(
        "llm_calls",
        "direct",
        "llm:<id>",
        "LLM prompt/response evidence",
    ),
    row(
        "message_recipients",
        "direct",
        "recipient:<event>:<pubkey>[:session]",
        "message delivery edges",
    ),
    row(
        "messages",
        "direct",
        "message:<id> | event:<id>",
        "canonical chat rows",
    ),
    row(
        "outbox",
        "direct",
        "outbox:<id>",
        "durable publish queue rows",
    ),
    row(
        "workspace_roots",
        "direct",
        "workspace:<channel>",
        "channel workspace bindings",
    ),
    row(
        "receipts",
        "direct",
        "receipt:<id> | explain handles",
        "Trellis receipt ledger rows",
    ),
    row(
        "relay_channel_member_sets",
        "aggregate",
        "membership_snapshot:<channel> | roster:<channel>",
        "membership snapshot high-water marks",
    ),
    row(
        "relay_channel_members",
        "direct",
        "member:<channel>:<pubkey> | admin:<channel>:<pubkey>",
        "member/admin rows",
    ),
    row(
        "relay_channels",
        "direct",
        "channel:<h> | readiness:<h>",
        "channel metadata and readiness",
    ),
    row(
        "relay_agent_roster",
        "aggregate",
        "awareness:<channel> | who:<channel>",
        "backend capability advertisements",
    ),
    row(
        "relay_event_quarantine",
        "direct",
        "quarantine:<event>",
        "quarantined relay events",
    ),
    row(
        "relay_events",
        "direct",
        "event:<id>",
        "materialized relay events",
    ),
    row(
        "relay_profiles",
        "direct",
        "profile:<pubkey> | pubkey:<pubkey>",
        "profile cache rows",
    ),
    row(
        "relay_status",
        "direct",
        "status:<session>",
        "published agent status rows",
    ),
    row(
        "session_aliases",
        "direct",
        "alias:<harness>:<kind>:<value>",
        "external-to-canonical session aliases",
    ),
    row(
        "session_claims",
        "aggregate",
        "who:<channel> | awareness:<channel>",
        "ephemeral route claims and dormant presence",
    ),
    row(
        "session_channels",
        "direct",
        "joined:<session>[:channel]",
        "joined channel bindings",
    ),
    row(
        "sessions",
        "direct",
        "session:<session>",
        "local hosted session rows",
    ),
    row(
        "trellis_commits",
        "direct",
        "commit:<id> | txn:<surface>:<id>",
        "Trellis transaction commits",
    ),
    row(
        "trellis_replay_capsules",
        "direct",
        "capsule:<id>",
        "captured replay scripts",
    ),
];

const fn row(
    table: &'static str,
    mode: &'static str,
    targets: &'static str,
    proves: &'static str,
) -> TableCoverage {
    TableCoverage {
        table,
        mode,
        targets,
        proves,
    }
}

pub(super) fn table_coverage(table: &str) -> Option<&'static TableCoverage> {
    DURABLE_TABLES.iter().find(|row| row.table == table)
}

pub(super) fn surface_targets(surface: &str) -> &'static str {
    match surface {
        "status" => "status:<session>",
        "subscriptions" => "sub:<channel> | sub/<h|d|p>/<id>",
        "hook_context" => "hook:<session> | hook_context:<session>",
        "turn_lifecycle" => "turn:<session> | turn_lifecycle:<session>",
        "cursor" => "cursor:<session> | cur:<session>",
        "session_start" => "session_start:<session>",
        "session_watch" => "watch:<session> | session_watch:<session>",
        "outbox" => "outbox:<id>",
        _ => "",
    }
}
