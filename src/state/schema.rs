pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    session_id    TEXT PRIMARY KEY,
    agent_slug    TEXT NOT NULL,
    agent_pubkey  TEXT NOT NULL,
    project       TEXT NOT NULL,
    host          TEXT NOT NULL,
    child_pid     INTEGER,
    watch_pid     INTEGER,
    created_at    INTEGER NOT NULL,
    last_seen     INTEGER NOT NULL DEFAULT 0,
    transcript_path TEXT,
    alive         INTEGER NOT NULL DEFAULT 1,
    rel_cwd       TEXT NOT NULL DEFAULT '',
    channel       TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS profiles (
    pubkey     TEXT PRIMARY KEY,
    slug       TEXT NOT NULL,
    host       TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    is_backend INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS peer_sessions (
    session_id TEXT PRIMARY KEY,
    pubkey     TEXT NOT NULL,
    slug       TEXT NOT NULL,
    project    TEXT NOT NULL,
    host       TEXT NOT NULL,
    last_seen  INTEGER NOT NULL,
    first_seen INTEGER NOT NULL DEFAULT 0,
    rel_cwd    TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS chat_inbox (
    chat_event_id     TEXT NOT NULL,
    target_session    TEXT NOT NULL,
    from_pubkey       TEXT NOT NULL,
    from_slug         TEXT NOT NULL,
    project           TEXT NOT NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    delivered         INTEGER NOT NULL DEFAULT 0,
    delivered_at      INTEGER NOT NULL DEFAULT 0,
    notified_at       INTEGER NOT NULL DEFAULT 0,
    from_session      TEXT NOT NULL DEFAULT '',
    mentioned_session TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (chat_event_id, target_session)
);
CREATE TABLE IF NOT EXISTS chat_messages (
    chat_event_id     TEXT PRIMARY KEY,
    from_pubkey       TEXT NOT NULL,
    from_slug         TEXT NOT NULL,
    host              TEXT NOT NULL DEFAULT '',
    project           TEXT NOT NULL,
    body              TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    from_session      TEXT NOT NULL DEFAULT '',
    mentioned_session TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_chat_messages_project_created
    ON chat_messages(project, created_at, chat_event_id);
-- Per-session turn state: flipped by the host's turn-start/turn-end hooks. The
-- engine polls this to decide when to distill activity (30s into a turn, then
-- every few minutes) and when to go idle. No tool events — distillation reads
-- the conversation transcript, not tool names.
CREATE TABLE IF NOT EXISTS turn_state (
    session_id      TEXT PRIMARY KEY,
    working         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0,
    -- Mid-turn delta cursor: timestamp of the last PostToolUse turn_check.
    -- Reset to 0 at turn start so each in-turn check reports only sibling
    -- changes since the previous check (the guarded ALTER below migrates
    -- pre-existing on-disk databases that predate this column).
    last_check_at   INTEGER NOT NULL DEFAULT 0
);
-- ── canonical session aggregate (single source of truth) ─────────────────────
-- ONE row per local session keyed by the daemon-minted canonical session_id.
-- Holds the whole public fact (title/activity/busy/phase/turn/lifecycle) plus
-- the liveness clock (last_seen) and the delta cursors (first_seen set ONLY on
-- insert, updated_at bumped in lockstep with state_version on every public
-- content change — NEVER on a bare heartbeat). All mutation flows through the
-- Store transition methods, each one txn that bumps state_version and enqueues a
-- status_outbox row when public status changed.
CREATE TABLE IF NOT EXISTS session_state (
    session_id      TEXT PRIMARY KEY,
    agent_slug      TEXT NOT NULL,
    agent_pubkey    TEXT NOT NULL,
    project         TEXT NOT NULL,
    host            TEXT NOT NULL,
    rel_cwd         TEXT NOT NULL DEFAULT '',
    title           TEXT NOT NULL DEFAULT '',
    title_source    TEXT NOT NULL DEFAULT 'none',
    activity        TEXT NOT NULL DEFAULT '',
    busy            INTEGER NOT NULL DEFAULT 0,
    phase           TEXT NOT NULL DEFAULT 'idle',
    turn_id         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0,
    last_distill_at INTEGER NOT NULL DEFAULT 0,
    last_seen       INTEGER NOT NULL DEFAULT 0,
    resume_id       TEXT NOT NULL DEFAULT '',
    state_version   INTEGER NOT NULL DEFAULT 0,
    lifecycle       TEXT NOT NULL DEFAULT 'active',
    first_seen      INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_session_state_project_seen
    ON session_state(project, last_seen);
CREATE INDEX IF NOT EXISTS idx_session_state_project_updated
    ON session_state(project, updated_at);
-- Maps every external identifier (harness-native id, resume token, tmux pane,
-- watch pid, generated te-* id) to the canonical session_id. (harness,
-- external_id_kind, external_id) is the PK so the same raw id under two harnesses
-- or two kinds never collide.
CREATE TABLE IF NOT EXISTS session_aliases (
    harness          TEXT NOT NULL,
    external_id_kind TEXT NOT NULL,
    external_id      TEXT NOT NULL,
    session_id       TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    PRIMARY KEY (harness, external_id_kind, external_id)
);
CREATE INDEX IF NOT EXISTS idx_session_aliases_session
    ON session_aliases(session_id);
-- Peer mirror, materialized from inbound kind:30315. Keyed by (pubkey, project):
-- one row per agent per group. `project` == the kind:30315 `d` tag == `h` tag ==
-- project slug. A newer heartbeat replaces the older row for the same agent+group.
-- Same delta cursors as session_state so status_delta_since works across both.
-- last_seen = the event's emitted-at (a finished peer stops emitting → ages out);
-- never local-writable. No native_session_id — issue #5 §4.
CREATE TABLE IF NOT EXISTS peer_session_state (
    pubkey            TEXT NOT NULL,
    project           TEXT NOT NULL,
    agent_slug        TEXT NOT NULL DEFAULT '',
    host              TEXT NOT NULL DEFAULT '',
    rel_cwd           TEXT NOT NULL DEFAULT '',
    title             TEXT NOT NULL DEFAULT '',
    activity          TEXT NOT NULL DEFAULT '',
    busy              INTEGER NOT NULL DEFAULT 0,
    last_seen         INTEGER NOT NULL DEFAULT 0,
    state_version     INTEGER NOT NULL DEFAULT 0,
    lifecycle         TEXT NOT NULL DEFAULT 'active',
    first_seen        INTEGER NOT NULL DEFAULT 0,
    updated_at        INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, project)
);
CREATE INDEX IF NOT EXISTS idx_peer_session_state_project_seen
    ON peer_session_state(project, last_seen);
-- Relay-confirmed published actor presence. This is the cohesive read model for
-- kind:30315 status events observed on or accepted by the relay. Local runtime
-- tables keep only local process/draft state; consumers that need published
-- peer/echo state read this projection instead of a peer-specific table.
CREATE TABLE IF NOT EXISTS presence_state (
    pubkey          TEXT NOT NULL,
    project         TEXT NOT NULL,
    local_session_id TEXT NOT NULL DEFAULT '',
    agent_slug      TEXT NOT NULL DEFAULT '',
    host            TEXT NOT NULL DEFAULT '',
    rel_cwd         TEXT NOT NULL DEFAULT '',
    title           TEXT NOT NULL DEFAULT '',
    title_source    TEXT NOT NULL DEFAULT 'peer',
    activity        TEXT NOT NULL DEFAULT '',
    busy            INTEGER NOT NULL DEFAULT 0,
    phase           TEXT NOT NULL DEFAULT 'idle',
    turn_id         INTEGER NOT NULL DEFAULT 0,
    turn_started_at INTEGER NOT NULL DEFAULT 0,
    last_distill_at INTEGER NOT NULL DEFAULT 0,
    last_seen       INTEGER NOT NULL DEFAULT 0,
    resume_id       TEXT NOT NULL DEFAULT '',
    state_version   INTEGER NOT NULL DEFAULT 0,
    lifecycle       TEXT NOT NULL DEFAULT 'active',
    first_seen      INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL DEFAULT 0,
    native_event_id TEXT NOT NULL DEFAULT '',
    confirmed_at    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, project)
);
CREATE INDEX IF NOT EXISTS idx_presence_state_project_seen
    ON presence_state(project, last_seen);
CREATE INDEX IF NOT EXISTS idx_presence_state_project_updated
    ON presence_state(project, updated_at);
-- Desired kind:30315 publications. One row per (session_id, state_version): the
-- daemon drainer publishes it via Nip29Provider::set_status, records the
-- native event id, and retries on failure. Only versioned CONTENT changes land
-- here; the per-heartbeat liveness re-arm republishes the latest snapshot WITHOUT
-- an outbox row.
CREATE TABLE IF NOT EXISTS status_outbox (
    session_id      TEXT NOT NULL,
    state_version   INTEGER NOT NULL,
    publish_state   TEXT NOT NULL DEFAULT 'pending',
    native_event_id TEXT,
    retries         INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,
    enqueued_at     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, state_version)
);
CREATE INDEX IF NOT EXISTS idx_status_outbox_pending
    ON status_outbox(publish_state, enqueued_at);
-- NIP-29 group metadata cache: the 'about' text for each project channel (kind 39000).
CREATE TABLE IF NOT EXISTS project_meta (
    project    TEXT PRIMARY KEY,
    about      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    -- NIP-29 subgroup hierarchy (issue #3): `name` is the human display name from
    -- the relay-authored kind:39000 `name` tag; `parent` is the parent group id
    -- from its `parent` tag (empty for top-level project groups). Lets
    -- `groups list` render the tree from local state without hitting the relay.
    name       TEXT NOT NULL DEFAULT '',
    parent     TEXT NOT NULL DEFAULT ''
);
-- NIP-29 groups this daemon owns/manages (created + locked closed via tenexPrivateKey).
CREATE TABLE IF NOT EXISTS owned_groups (
    project    TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL,
    -- 1 when this group is actually owned/managed by this daemon. Session-room
    -- rows may exist with owns_group=0 when a no-management-key daemon starts
    -- fail-open and needs local routing metadata without claiming relay admin.
    owns_group INTEGER NOT NULL DEFAULT 1,
    -- 1 when this owned group is a per-session room (issue #6), so only the
    -- owning session auto-renames it to its distilled title. The ALTER in `open`
    -- backfills this column for databases created before the column existed.
    is_session_room INTEGER NOT NULL DEFAULT 0,
    -- The work-root project a per-session room is nested under. Set at mint and
    -- NOT touched by the relay materializer (unlike project_meta.parent, which a
    -- relay that doesn't re-emit the NIP-29 parent tag can clobber). Lets
    -- host-side resolution find a session by its work-root now that the room id
    -- (session-<hash>) no longer encodes the project name.
    room_parent TEXT NOT NULL DEFAULT ''
);
-- NIP-29 group membership cache (relay-authoritative kind 39002 + our optimistic
-- put-user writes). Lets session_start skip redundant 9000 publishes idempotently.
CREATE TABLE IF NOT EXISTS group_members (
    project    TEXT NOT NULL,
    pubkey     TEXT NOT NULL,
    role       TEXT NOT NULL DEFAULT 'member',
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (project, pubkey)
);
-- Durable dedup for subgroup add-agents orchestration events (issue #3). The
-- relay redelivers the same kind:9 on every matching subscription, and a daemon
-- restart replays history; this table makes provisioning fire AT MOST ONCE per
-- event id, surviving restarts (unlike the in-memory first_sight cache).
CREATE TABLE IF NOT EXISTS processed_orchestration (
    event_id     TEXT PRIMARY KEY,
    processed_at INTEGER NOT NULL
);

-- ── Phase 1: canonical read-model tables ──────────────────────────────────────
-- Durable project identities with surrogate ids; origin tables map fabric
-- coordinates back to local ids.
CREATE TABLE IF NOT EXISTS projects (
    project_id   TEXT PRIMARY KEY,
    display_slug TEXT NOT NULL,
    about        TEXT,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS project_origins (
    project_id           TEXT NOT NULL,
    fabric               TEXT NOT NULL,
    provider_instance    TEXT NOT NULL,
    native_project_key   TEXT NOT NULL,
    UNIQUE(fabric, provider_instance, native_project_key)
);
CREATE TABLE IF NOT EXISTS inbound_quarantine (
    native_event_id TEXT PRIMARY KEY,
    project_id      TEXT,
    reason          TEXT NOT NULL,
    raw_envelope    TEXT NOT NULL,
    created_at      INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS membership (
    project_id  TEXT NOT NULL,
    pubkey      TEXT NOT NULL,
    role        TEXT NOT NULL,
    admitted_at INTEGER NOT NULL,
    revoked_at  INTEGER,
    source      TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY(project_id, pubkey)
);
-- Per-session distillation error log. Written by the runtime when the LLM
-- distiller fails; read by rpc_statusline to flash a warning. One row per
-- session (upsert) so only the last error is kept — the log file has full history.
CREATE TABLE IF NOT EXISTS session_errors (
    session_id TEXT PRIMARY KEY,
    message    TEXT NOT NULL,
    ts         INTEGER NOT NULL
);
-- TMUX control-plane: one row per (session, kind='tmux') endpoint. Written by
-- rpc_session_start when the hook env supplies TMUX_PANE; read by the pending
-- message dispatcher. `target` is the stable tmux pane id (e.g. '%5'). `meta` is a JSON
-- object that may carry {"socket":"...", "pane_command":"claude"}.
CREATE TABLE IF NOT EXISTS session_endpoints (
    session_id    TEXT NOT NULL,
    kind          TEXT NOT NULL,
    target        TEXT NOT NULL,
    meta          TEXT NOT NULL DEFAULT '',
    registered_at INTEGER NOT NULL,
    last_verified INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (session_id, kind)
);
-- Absolute project path indexed by project slug. Populated by session_start so
-- the tmux spawn command knows where to cd.
CREATE TABLE IF NOT EXISTS project_paths (
    project    TEXT PRIMARY KEY,
    abs_path   TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
-- Stage 3 (Issue #2): derived per-session Nostr pubkeys. Maps the pubkey that
-- results from `identity::derive_session_keys` back to the owning session.
-- Populated on session_start; cleared on session_end / engine self-exit /
-- crash-GC. Used by two subsystems:
--   1. Routing: a mention p-tagged to a session pubkey resolves to the owning
--      session via `session_pubkey_info` in `route_mention_into_with_id`.
--   2. Slug resolution: `resolve_slug_for_pubkey(session_pubkey)` fabricates
--      "<codename> (<agent_slug>)" from this table so inbound session-signed
--      events render a sensible sender name without a round-trip to the relay.
CREATE TABLE IF NOT EXISTS session_pubkeys (
    session_pubkey  TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL,
    agent_pubkey    TEXT NOT NULL,
    agent_slug      TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_pubkeys_session
    ON session_pubkeys(session_id);
-- Durable ordinal identities (issue #47). A `(base_pubkey, ordinal)` pair maps
-- to one stable pubkey reused across rooms; ordinal 0 is the base agent key.
-- This is the local inventory of every ordinal ever allocated, so the
-- subscription `#p` set is bounded by (agents × concurrency), not sessions.
CREATE TABLE IF NOT EXISTS agent_ordinals (
    base_pubkey TEXT NOT NULL,
    agent_slug  TEXT NOT NULL,
    ordinal     INTEGER NOT NULL,
    pubkey      TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (base_pubkey, ordinal)
);
-- Route + resume binding (issue #47). The authoritative resume key is
-- (pubkey, h), where h is route_scope (channel when set, else per-session room).
-- The native harness session id is a STORED ATTRIBUTE here (inverting the old
-- derive-keys-from-anchor model): a mention to an offline ordinal resumes its
-- bound native session. The same ordinal pubkey may be live in several rooms
-- (one row per (pubkey,h)) but never twice in one room. A row persists with
-- alive=0 after the session dies so a future mention can resume it.
CREATE TABLE IF NOT EXISTS session_identity_routes (
    pubkey       TEXT NOT NULL,
    h            TEXT NOT NULL,
    session_id   TEXT NOT NULL,
    base_pubkey  TEXT NOT NULL,
    agent_slug   TEXT NOT NULL,
    ordinal      INTEGER NOT NULL,
    label        TEXT NOT NULL,
    harness_kind TEXT NOT NULL DEFAULT '',
    native_id    TEXT NOT NULL DEFAULT '',
    alive        INTEGER NOT NULL DEFAULT 0,
    bound_at     INTEGER NOT NULL,
    released_at  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (pubkey, h),
    UNIQUE (session_id)
);
CREATE INDEX IF NOT EXISTS idx_identity_routes_session
    ON session_identity_routes(session_id);
CREATE INDEX IF NOT EXISTS idx_identity_routes_base
    ON session_identity_routes(base_pubkey, h);
"#;
