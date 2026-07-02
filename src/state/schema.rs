//! The stamped persistence schema (no backwards-compatible migrations).
//!
//! Six `relay_*` tables are materialized caches and may be dropped/rebuilt from
//! relay state. The remaining local tables are non-rebuildable daemon state:
//! session bindings, aliases, derived identities, inbox/outbox ledgers, and
//! project path mappings. A pubkey appears AT MOST ONCE per channel (enforced via
//! primary key).

pub(super) const SCHEMA_VERSION: u32 = 1;

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
    resume_id         TEXT NOT NULL DEFAULT ''
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
    pubkey       TEXT PRIMARY KEY,
    base_pubkey  TEXT NOT NULL,
    agent_slug   TEXT NOT NULL DEFAULT '',
    ordinal      INTEGER NOT NULL DEFAULT 0,
    session_id   TEXT NOT NULL DEFAULT '',
    channel_h    TEXT NOT NULL DEFAULT '',
    native_id    TEXT NOT NULL DEFAULT '',
    alive        INTEGER NOT NULL DEFAULT 0,
    created_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_identities_base
    ON identities(base_pubkey, channel_h);
CREATE UNIQUE INDEX IF NOT EXISTS idx_identities_session
    ON identities(session_id) WHERE session_id <> '';

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
    enqueued_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_outbox_pending
    ON outbox(state, local_id);

CREATE TABLE IF NOT EXISTS project_roots (
    channel_h   TEXT PRIMARY KEY,
    abs_path    TEXT NOT NULL,
    updated_at  INTEGER NOT NULL
);
"#;
