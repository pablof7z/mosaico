//! The core schema DDL, split out of `schema.rs` to keep that file small.
pub(in crate::state::schema) mod operational;

pub(super) const SCHEMA_PARTS: &[&str] = &[
    CORE_SCHEMA,
    operational::OPERATIONAL_SCHEMA,
    operational::NATIVE_TURN_SCHEMA,
];

const CORE_SCHEMA: &str = r#"
-- ── local state (facts the relay can't carry) ────────────────────────────────

CREATE TABLE IF NOT EXISTS sessions (
    pubkey             TEXT PRIMARY KEY,
    runtime_generation INTEGER NOT NULL,
    agent_slug        TEXT NOT NULL DEFAULT '',
    work_root         TEXT NOT NULL DEFAULT '',
    readiness_parent  TEXT NOT NULL DEFAULT '',
    observed_harness  TEXT NOT NULL DEFAULT '',
    claimed_harness   TEXT NOT NULL DEFAULT '',
    admitted_bundle   TEXT NOT NULL DEFAULT '',
    admitted_transport TEXT NOT NULL DEFAULT ''
        CHECK (admitted_transport IN ('', 'pty', 'acp', 'app-server')),
    endpoint_provenance TEXT NOT NULL DEFAULT ''
        CHECK (endpoint_provenance IN ('', 'launch', 'hook', 'migration')),
    child_pid         INTEGER,
    runtime_state     TEXT NOT NULL DEFAULT 'running'
        CHECK (runtime_state IN ('running', 'stopping', 'stopped')),
    presentation_state TEXT NOT NULL DEFAULT 'unavailable'
        CHECK (presentation_state IN ('unavailable', 'headed', 'headless')),
    work_state        TEXT NOT NULL DEFAULT 'idle'
        CHECK (work_state IN ('idle', 'working')),
    recovery_state    TEXT NOT NULL DEFAULT 'pending'
        CHECK (recovery_state IN ('pending', 'ready', 'revoked')),
    lifecycle_epoch   INTEGER NOT NULL DEFAULT 1,
    attachment_epoch  INTEGER NOT NULL DEFAULT 0,
    idle_since        INTEGER NOT NULL DEFAULT 0,
    idle_deadline     INTEGER NOT NULL DEFAULT 0,
    stopped_at        INTEGER NOT NULL DEFAULT 0,
    stop_reason       TEXT CHECK (stop_reason IS NULL OR stop_reason IN (
        'unknown', 'attached_clean_exit', 'idle_evicted', 'headless_exit',
        'crash', 'operator_kill', 'revoked', 'superseded'
    )),
    turn_count        INTEGER NOT NULL DEFAULT 0,
    busy_seconds      INTEGER NOT NULL DEFAULT 0,
    created_at        INTEGER NOT NULL,
    last_seen         INTEGER NOT NULL DEFAULT 0,
    turn_started_at   INTEGER NOT NULL DEFAULT 0,
    seen_cursor       INTEGER NOT NULL DEFAULT 0,
    title             TEXT NOT NULL DEFAULT '',
    state_changed_at  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_runtime
    ON sessions(runtime_state);
CREATE INDEX IF NOT EXISTS idx_sessions_idle_deadline
    ON sessions(runtime_state, presentation_state, work_state, idle_deadline);

-- Generation-scoped progressive coaching already emitted to one session.
CREATE TABLE IF NOT EXISTS session_coaching (
    pubkey             TEXT NOT NULL,
    runtime_generation INTEGER NOT NULL CHECK (runtime_generation > 0),
    code               TEXT NOT NULL CHECK (code <> ''),
    shown_at           INTEGER NOT NULL,
    PRIMARY KEY (pubkey, runtime_generation, code)
);

-- Keyed, non-raw correlation aliases for remote MCP conversation actors.
CREATE TABLE IF NOT EXISTS mcp_actor_aliases (
    actor_key  TEXT PRIMARY KEY,
    actor_kind TEXT NOT NULL CHECK (actor_kind IN ('openai', 'grok')),
    pubkey     TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL,
    last_seen  INTEGER NOT NULL
);

-- Durable exact-session channel membership. Automatic delivery is fenced by
-- both the signed event time and the local event-arrival sequence at join.
-- Fabric standing is owned exclusively by session_standing.
CREATE TABLE IF NOT EXISTS session_channels (
    pubkey           TEXT NOT NULL,
    channel_h       TEXT NOT NULL,
    joined_at       INTEGER NOT NULL,
    joined_event_seq INTEGER NOT NULL,
    PRIMARY KEY (pubkey, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_session_channels_channel
    ON session_channels(channel_h, pubkey);

-- Minimal durable cursor: event contents remain exclusively in NMP.
CREATE TABLE IF NOT EXISTS nmp_event_arrivals (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE
);

-- NMP owns the event and attachment metadata. Mosaico retains only the
-- verified directory containing host-local downloaded files.
CREATE TABLE IF NOT EXISTS message_attachments (
    event_id  TEXT PRIMARY KEY,
    directory TEXT NOT NULL CHECK (directory <> '')
);

CREATE TABLE IF NOT EXISTS session_standing (
    pubkey                  TEXT NOT NULL,
    channel_h               TEXT NOT NULL,
    state                   TEXT NOT NULL CHECK (state IN ('member', 'absent')),
    standing_epoch          INTEGER NOT NULL DEFAULT 1,
    session_lifecycle_epoch INTEGER NOT NULL,
    updated_at              INTEGER NOT NULL,
    PRIMARY KEY (pubkey, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_session_standing_state
    ON session_standing(state, pubkey, channel_h);

CREATE TABLE IF NOT EXISTS session_locators (
    harness        TEXT NOT NULL,
    locator_kind   TEXT NOT NULL
        CHECK (locator_kind IN ('native_resume', 'pty', 'acp', 'app_server', 'pid')),
    locator_value  TEXT NOT NULL,
    pubkey         TEXT NOT NULL,
    runtime_generation INTEGER NOT NULL DEFAULT 0,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (harness, locator_kind, locator_value)
);
CREATE INDEX IF NOT EXISTS idx_session_locators_pubkey
    ON session_locators(pubkey);
CREATE INDEX IF NOT EXISTS idx_session_locators_value
    ON session_locators(locator_value);
CREATE UNIQUE INDEX IF NOT EXISTS idx_session_locators_native_resume
    ON session_locators(pubkey) WHERE locator_kind='native_resume';
CREATE UNIQUE INDEX IF NOT EXISTS idx_session_locators_runtime_endpoint
    ON session_locators(pubkey, harness, locator_kind)
    WHERE locator_kind IN ('pty', 'acp', 'app_server', 'pid');

CREATE TABLE IF NOT EXISTS session_signers (pubkey TEXT PRIMARY KEY, signer_salt TEXT NOT NULL);

CREATE TABLE IF NOT EXISTS handle_leases (
    handle          TEXT PRIMARY KEY,
    pubkey          TEXT NOT NULL UNIQUE,
    agent_slug      TEXT NOT NULL,
    leased_at       INTEGER NOT NULL,
    last_active_at  INTEGER NOT NULL,
    live            INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_handle_leases_reclaim
    ON handle_leases(agent_slug, live, last_active_at);

CREATE TABLE IF NOT EXISTS inbox (
    event_id        TEXT NOT NULL,
    target_pubkey   TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending',
    from_pubkey     TEXT NOT NULL DEFAULT '',
    channel_h       TEXT NOT NULL DEFAULT '',
    body            TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    delivered_at    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, target_pubkey)
);
CREATE INDEX IF NOT EXISTS idx_inbox_pending
    ON inbox(target_pubkey, state, created_at);

CREATE TABLE IF NOT EXISTS event_claims (
    event_id       TEXT NOT NULL,
    claim_key      TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'pending',
    from_pubkey    TEXT NOT NULL DEFAULT '',
    channel_h      TEXT NOT NULL DEFAULT '',
    body           TEXT NOT NULL DEFAULT '',
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, claim_key)
);
CREATE INDEX IF NOT EXISTS idx_event_claims_state
    ON event_claims(state, updated_at);

CREATE TABLE IF NOT EXISTS workspace_roots (
    channel_h   TEXT PRIMARY KEY,
    abs_path    TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS channel_resolution_intents (
    parent      TEXT NOT NULL,
    name        TEXT NOT NULL,
    channel_h   TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (parent, name)
);
"#;
