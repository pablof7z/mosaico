//! The raw schema DDL, split out of `schema.rs` to keep that file small.
pub(super) const SCHEMA: &str = r#"
-- ── relay_* materialized caches (drop & rebuild from relay anytime) ───────────

CREATE TABLE IF NOT EXISTS relay_channels (
    channel_h   TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    about       TEXT NOT NULL DEFAULT '',
    parent      TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS relay_channel_members (
    channel_h   TEXT NOT NULL,
    pubkey      TEXT NOT NULL,
    role        TEXT NOT NULL DEFAULT 'member',
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (channel_h, pubkey)
);
CREATE INDEX IF NOT EXISTS idx_relay_channel_members_pubkey
    ON relay_channel_members(pubkey, role);

CREATE TABLE IF NOT EXISTS relay_channel_member_sets (
    channel_h   TEXT NOT NULL,
    role        TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (channel_h, role)
);

CREATE TABLE IF NOT EXISTS relay_profiles (
    pubkey      TEXT PRIMARY KEY,
    name        TEXT NOT NULL DEFAULT '',
    slug        TEXT NOT NULL DEFAULT '',
    agent_slug  TEXT NOT NULL DEFAULT '',
    host        TEXT NOT NULL DEFAULT '',
    is_backend  INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS relay_status (
    pubkey       TEXT NOT NULL,
    session_id   TEXT NOT NULL DEFAULT '',
    channel_h    TEXT NOT NULL,
    slug         TEXT NOT NULL DEFAULT '',
    title        TEXT NOT NULL DEFAULT '',
    activity     TEXT NOT NULL DEFAULT '',
    busy         INTEGER NOT NULL DEFAULT 0,
    last_seen    INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER NOT NULL DEFAULT 0,
    expiration   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, session_id, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_relay_status_channel
    ON relay_status(channel_h, expiration);
CREATE INDEX IF NOT EXISTS idx_relay_status_session
    ON relay_status(pubkey, session_id);

CREATE TABLE IF NOT EXISTS relay_agent_roster (
    backend_pubkey TEXT NOT NULL,
    agent_slug     TEXT NOT NULL,
    channel_h      TEXT NOT NULL,
    host           TEXT NOT NULL DEFAULT '',
    use_criteria   TEXT NOT NULL DEFAULT '',
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (backend_pubkey, agent_slug, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_relay_agent_roster_channel
    ON relay_agent_roster(channel_h, host, agent_slug);
CREATE INDEX IF NOT EXISTS idx_relay_agent_roster_backend
    ON relay_agent_roster(backend_pubkey, agent_slug);

CREATE TABLE IF NOT EXISTS relay_events (
    id          TEXT PRIMARY KEY,
    kind        INTEGER NOT NULL,
    pubkey      TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    channel_h   TEXT NOT NULL DEFAULT '',
    d_tag       TEXT NOT NULL DEFAULT '',
    content     TEXT NOT NULL DEFAULT '',
    tags_json   TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_relay_events_channel
    ON relay_events(channel_h, created_at, id);
CREATE INDEX IF NOT EXISTS idx_relay_events_kind
    ON relay_events(kind);
CREATE INDEX IF NOT EXISTS idx_relay_events_addr
    ON relay_events(kind, pubkey, d_tag);

CREATE TABLE IF NOT EXISTS relay_event_quarantine (
    id             TEXT PRIMARY KEY,
    kind           INTEGER NOT NULL,
    pubkey         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    channel_h      TEXT NOT NULL DEFAULT '',
    event_json     TEXT NOT NULL,
    reason         TEXT NOT NULL DEFAULT '',
    quarantined_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_relay_event_quarantine_channel
    ON relay_event_quarantine(channel_h, kind, created_at, id);

CREATE TABLE IF NOT EXISTS messages (
    message_id      TEXT PRIMARY KEY,
    thread_id       TEXT NOT NULL DEFAULT '',
    channel_h       TEXT NOT NULL,
    author_pubkey   TEXT NOT NULL,
    author_session  TEXT,
    body            TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    direction       TEXT NOT NULL DEFAULT 'inbound',
    sync_state      TEXT NOT NULL DEFAULT 'accepted',
    native_event_id TEXT,
    error           TEXT
);
CREATE INDEX IF NOT EXISTS idx_messages_channel
    ON messages(channel_h, created_at, message_id);
CREATE INDEX IF NOT EXISTS idx_messages_native
    ON messages(native_event_id);
CREATE INDEX IF NOT EXISTS idx_messages_author_session
    ON messages(author_session, direction, sync_state, created_at);

CREATE TABLE IF NOT EXISTS message_recipients (
    message_id       TEXT NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    target_session   TEXT NOT NULL DEFAULT '',
    delivered_at     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_id, recipient_pubkey, target_session)
);
CREATE INDEX IF NOT EXISTS idx_message_recipients_target
    ON message_recipients(target_session, delivered_at);

-- ── local state (facts the relay can't carry) ────────────────────────────────

CREATE TABLE IF NOT EXISTS sessions (
    session_id        TEXT PRIMARY KEY,
    agent_pubkey      TEXT NOT NULL,
    agent_slug        TEXT NOT NULL DEFAULT '',
    channel_h         TEXT NOT NULL DEFAULT '',
    harness           TEXT NOT NULL DEFAULT '',
    child_pid         INTEGER,
    transcript_path   TEXT,
    alive             INTEGER NOT NULL DEFAULT 1,
    created_at        INTEGER NOT NULL,
    last_seen         INTEGER NOT NULL DEFAULT 0,
    working           INTEGER NOT NULL DEFAULT 0,
    turn_started_at   INTEGER NOT NULL DEFAULT 0,
    last_distill_at   INTEGER NOT NULL DEFAULT 0,
    seen_cursor       INTEGER NOT NULL DEFAULT 0,
    title             TEXT NOT NULL DEFAULT '',
    activity          TEXT NOT NULL DEFAULT '',
    resume_id         TEXT NOT NULL DEFAULT '',
    distill_fail_streak INTEGER NOT NULL DEFAULT 0,
    distill_notice_at   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_alive
    ON sessions(alive, channel_h);

CREATE TABLE IF NOT EXISTS session_channels (
    session_id   TEXT NOT NULL,
    channel_h    TEXT NOT NULL,
    joined_at    INTEGER NOT NULL,
    PRIMARY KEY (session_id, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_session_channels_channel
    ON session_channels(channel_h, session_id);

CREATE TABLE IF NOT EXISTS session_aliases (
    harness           TEXT NOT NULL,
    external_id_kind  TEXT NOT NULL,
    external_id       TEXT NOT NULL,
    session_id        TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    PRIMARY KEY (harness, external_id_kind, external_id)
);
CREATE INDEX IF NOT EXISTS idx_session_aliases_session
    ON session_aliases(session_id);
CREATE INDEX IF NOT EXISTS idx_session_aliases_external
    ON session_aliases(external_id);

CREATE TABLE IF NOT EXISTS identities (
    pubkey       TEXT NOT NULL,
    agent_slug   TEXT NOT NULL DEFAULT '',
    codename     TEXT NOT NULL DEFAULT '',
    session_id   TEXT NOT NULL DEFAULT '',
    channel_h    TEXT NOT NULL DEFAULT '',
    native_id    TEXT NOT NULL DEFAULT '',
    alive        INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL,
    PRIMARY KEY (pubkey, session_id)
);
CREATE INDEX IF NOT EXISTS idx_identities_channel
    ON identities(channel_h);
CREATE UNIQUE INDEX IF NOT EXISTS idx_identities_session
    ON identities(session_id) WHERE session_id <> '';

CREATE TABLE IF NOT EXISTS session_claims (pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '', codename TEXT NOT NULL DEFAULT '', session_id TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL DEFAULT '', native_id TEXT NOT NULL DEFAULT '', harness TEXT NOT NULL DEFAULT '', last_active_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, owner_backend_pubkey TEXT NOT NULL DEFAULT '', owner_host TEXT NOT NULL DEFAULT '', PRIMARY KEY (pubkey, channel_h));
CREATE INDEX IF NOT EXISTS idx_session_claims_expires ON session_claims(expires_at);
CREATE INDEX IF NOT EXISTS idx_session_claims_session ON session_claims(session_id);

CREATE TABLE IF NOT EXISTS inbox (
    event_id        TEXT NOT NULL,
    target_session  TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending',
    from_pubkey     TEXT NOT NULL DEFAULT '',
    channel_h       TEXT NOT NULL DEFAULT '',
    body            TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    delivered_at    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (event_id, target_session)
);
CREATE INDEX IF NOT EXISTS idx_inbox_pending
    ON inbox(target_session, state, created_at);

CREATE TABLE IF NOT EXISTS outbox (
    local_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    event_json   TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'pending',
    retries      INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    enqueued_at  INTEGER NOT NULL,
    -- Earliest wall-clock second this row may be (re)attempted. 0 = due now.
    -- Set to now+backoff on a failed publish so a wedged relay can't induce a
    -- retry storm; the drainer's peek gates on it.
    next_attempt_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON outbox(state, next_attempt_at, local_id);

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

CREATE TABLE IF NOT EXISTS channel_readiness_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, channel_h TEXT NOT NULL, expect_member TEXT NOT NULL DEFAULT '', parent_hint TEXT, name TEXT, source TEXT NOT NULL DEFAULT '', outcome TEXT NOT NULL DEFAULT '', reason TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_channel_readiness_attempts_channel ON channel_readiness_attempts(channel_h, created_at);
CREATE TABLE IF NOT EXISTS llm_calls (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id       TEXT NOT NULL,
    window_hash      TEXT NOT NULL,
    provider         TEXT NOT NULL,
    model            TEXT NOT NULL,
    system_prompt    TEXT NOT NULL,
    transcript_slice TEXT NOT NULL,
    raw_response     TEXT NOT NULL,
    parsed_title     TEXT, parsed_activity TEXT,
    created_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_calls_session ON llm_calls(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_llm_calls_window_hash ON llm_calls(window_hash);
CREATE TABLE IF NOT EXISTS receipts (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    surface          TEXT NOT NULL,
    transaction_id   INTEGER NOT NULL,
    revision         INTEGER NOT NULL,
    changed_summary  TEXT NOT NULL,
    commands         TEXT NOT NULL,
    artifact_ref     TEXT,
    created_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_receipts_surface ON receipts(surface, created_at);
CREATE INDEX IF NOT EXISTS idx_receipts_artifact_ref ON receipts(artifact_ref);
CREATE TABLE IF NOT EXISTS trellis_commits (id INTEGER PRIMARY KEY AUTOINCREMENT, surface TEXT NOT NULL, transaction_id INTEGER NOT NULL, revision INTEGER NOT NULL, mode TEXT NOT NULL DEFAULT '', trigger_kind TEXT NOT NULL, trigger_ref TEXT NOT NULL DEFAULT '', changed_inputs_json TEXT NOT NULL DEFAULT '[]', changed_derived_json TEXT NOT NULL DEFAULT '[]', changed_collections_json TEXT NOT NULL DEFAULT '[]', resource_commands_json TEXT NOT NULL DEFAULT '[]', output_frames_json TEXT NOT NULL DEFAULT '[]', command_count INTEGER NOT NULL DEFAULT 0, output_count INTEGER NOT NULL DEFAULT 0, effect_count INTEGER NOT NULL DEFAULT 0, suppressed_count INTEGER NOT NULL DEFAULT 0, noop INTEGER NOT NULL DEFAULT 0, oracle_status TEXT, oracle_error TEXT, duration_us INTEGER NOT NULL DEFAULT 0, graph_nodes INTEGER NOT NULL DEFAULT 0, graph_resources INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS idx_trellis_commits_surface ON trellis_commits(surface, created_at);
"#;
