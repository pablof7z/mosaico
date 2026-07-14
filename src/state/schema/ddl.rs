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
    channel_h    TEXT NOT NULL,
    slug         TEXT NOT NULL DEFAULT '',
    title        TEXT NOT NULL DEFAULT '',
    activity     TEXT NOT NULL DEFAULT '',
    busy         INTEGER NOT NULL DEFAULT 0,
    last_seen    INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER NOT NULL DEFAULT 0,
    expiration   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_relay_status_channel
    ON relay_status(channel_h, expiration);

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

-- NIP-25 reactions (kind:7) materialized from round-tripped relay events. A
-- reaction is passive awareness only: it is NEVER routed to inbox and never wakes
-- an agent. `reaction_id` is the kind:7 event id, so a relay echo of a locally
-- seeded reaction is idempotent.
CREATE TABLE IF NOT EXISTS relay_reactions (
    reaction_id       TEXT PRIMARY KEY,
    target_message_id TEXT NOT NULL,
    channel_h         TEXT NOT NULL DEFAULT '',
    reactor_pubkey    TEXT NOT NULL,
    emoji             TEXT NOT NULL DEFAULT '+',
    created_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_relay_reactions_target
    ON relay_reactions(target_message_id, created_at);

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
CREATE INDEX IF NOT EXISTS idx_messages_author_pubkey
    ON messages(author_pubkey, direction, sync_state, created_at);

CREATE TABLE IF NOT EXISTS message_recipients (
    message_id       TEXT NOT NULL,
    recipient_pubkey TEXT NOT NULL,
    delivered_at     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (message_id, recipient_pubkey)
);
CREATE INDEX IF NOT EXISTS idx_message_recipients_pubkey
    ON message_recipients(recipient_pubkey, delivered_at);

-- ── local state (facts the relay can't carry) ────────────────────────────────

CREATE TABLE IF NOT EXISTS sessions (
    pubkey             TEXT PRIMARY KEY,
    runtime_generation INTEGER NOT NULL,
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
    work_topic        TEXT NOT NULL DEFAULT '',
    work_topic_set_at INTEGER NOT NULL DEFAULT 0,
    seen_cursor       INTEGER NOT NULL DEFAULT 0,
    title             TEXT NOT NULL DEFAULT '',
    activity          TEXT NOT NULL DEFAULT '',
    distill_fail_streak INTEGER NOT NULL DEFAULT 0,
    distill_notice_at   INTEGER NOT NULL DEFAULT 0,
    explicit_chat_published_at INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sessions_alive
    ON sessions(alive, channel_h);

CREATE TABLE IF NOT EXISTS session_channels (
    pubkey        TEXT NOT NULL,
    channel_h    TEXT NOT NULL,
    joined_at    INTEGER NOT NULL,
    PRIMARY KEY (pubkey, channel_h)
);
CREATE INDEX IF NOT EXISTS idx_session_channels_channel
    ON session_channels(channel_h, pubkey);

CREATE TABLE IF NOT EXISTS session_locators (
    harness        TEXT NOT NULL,
    locator_kind   TEXT NOT NULL CHECK (locator_kind IN ('native_resume', 'pty', 'acp', 'pid')),
    locator_value  TEXT NOT NULL,
    pubkey         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    PRIMARY KEY (harness, locator_kind, locator_value)
);
CREATE INDEX IF NOT EXISTS idx_session_locators_pubkey
    ON session_locators(pubkey);
CREATE INDEX IF NOT EXISTS idx_session_locators_value
    ON session_locators(locator_value);
CREATE UNIQUE INDEX IF NOT EXISTS idx_session_locators_native_resume
    ON session_locators(pubkey) WHERE locator_kind='native_resume';

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

CREATE TABLE IF NOT EXISTS session_claims (pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL DEFAULT '', harness TEXT NOT NULL DEFAULT '', last_active_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, owner_backend_pubkey TEXT NOT NULL DEFAULT '', owner_host TEXT NOT NULL DEFAULT '', PRIMARY KEY (pubkey, channel_h));
CREATE INDEX IF NOT EXISTS idx_session_claims_expires ON session_claims(expires_at);

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
    pubkey           TEXT NOT NULL,
    window_hash      TEXT NOT NULL,
    provider         TEXT NOT NULL,
    model            TEXT NOT NULL,
    system_prompt    TEXT NOT NULL,
    transcript_slice TEXT NOT NULL,
    raw_response     TEXT NOT NULL,
    parsed_title     TEXT, parsed_activity TEXT,
    created_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_llm_calls_pubkey ON llm_calls(pubkey, created_at);
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

CREATE TABLE IF NOT EXISTS trellis_replay_capsules (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    surface        TEXT NOT NULL,
    trigger_kind   TEXT NOT NULL,
    trigger_ref    TEXT NOT NULL DEFAULT '',
    script_json    TEXT NOT NULL,
    script_bytes   INTEGER NOT NULL,
    format_version INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trellis_replay_capsules_surface
    ON trellis_replay_capsules(surface, created_at);
"#;
