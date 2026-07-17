use std::path::Path;

use rusqlite::Connection;

pub(super) fn create_schema_four(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(super::super::super::ddl::SCHEMA)
        .unwrap();
    conn.execute_batch(
        r#"
        DROP TABLE message_recipients;
        DROP TABLE messages;
        DROP TABLE session_locators;
        DROP TABLE session_channels;
        DROP TABLE session_claims;
        DROP TABLE sessions;
        DROP TABLE relay_status;

        CREATE TABLE messages (
            message_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL, author_pubkey TEXT NOT NULL,
            author_session TEXT, body TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL, direction TEXT NOT NULL DEFAULT 'inbound',
            sync_state TEXT NOT NULL DEFAULT 'accepted', native_event_id TEXT, error TEXT
        );
        CREATE INDEX idx_messages_channel ON messages(channel_h, created_at, message_id);
        CREATE INDEX idx_messages_native ON messages(native_event_id);
        CREATE INDEX idx_messages_author_session
            ON messages(author_session, direction, sync_state, created_at);
        CREATE TABLE message_recipients (
            message_id TEXT NOT NULL, recipient_pubkey TEXT NOT NULL,
            target_session TEXT NOT NULL DEFAULT '', delivered_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (message_id, recipient_pubkey, target_session)
        );
        CREATE INDEX idx_message_recipients_target
            ON message_recipients(target_session, delivered_at);

        CREATE TABLE sessions (
            session_id TEXT PRIMARY KEY, agent_pubkey TEXT NOT NULL,
            agent_slug TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '', child_pid INTEGER, transcript_path TEXT,
            alive INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
            last_seen INTEGER NOT NULL DEFAULT 0, working INTEGER NOT NULL DEFAULT 0,
            turn_started_at INTEGER NOT NULL DEFAULT 0,
            last_distill_at INTEGER NOT NULL DEFAULT 0, work_topic TEXT NOT NULL DEFAULT '',
            work_topic_set_at INTEGER NOT NULL DEFAULT 0, seen_cursor INTEGER NOT NULL DEFAULT 0,
            title TEXT NOT NULL DEFAULT '', activity TEXT NOT NULL DEFAULT '',
            resume_id TEXT NOT NULL DEFAULT '', distill_fail_streak INTEGER NOT NULL DEFAULT 0,
            distill_notice_at INTEGER NOT NULL DEFAULT 0,
            explicit_chat_published_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX idx_sessions_alive ON sessions(alive, channel_h);
        CREATE TABLE session_channels (
            session_id TEXT NOT NULL, channel_h TEXT NOT NULL, joined_at INTEGER NOT NULL,
            PRIMARY KEY (session_id, channel_h)
        );
        CREATE INDEX idx_session_channels_channel ON session_channels(channel_h, session_id);
        CREATE TABLE session_aliases (
            harness TEXT NOT NULL, external_id_kind TEXT NOT NULL,
            external_id TEXT NOT NULL, session_id TEXT NOT NULL, created_at INTEGER NOT NULL,
            PRIMARY KEY (harness, external_id_kind, external_id)
        );
        CREATE INDEX idx_session_aliases_session ON session_aliases(session_id);
        CREATE INDEX idx_session_aliases_external ON session_aliases(external_id);
        CREATE TABLE identities (
            pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '',
            codename TEXT NOT NULL DEFAULT '', session_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL DEFAULT '', native_id TEXT NOT NULL DEFAULT '',
            alive INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, session_id)
        );
        CREATE TABLE durable_agent_sessions (
            pubkey TEXT PRIMARY KEY, agent_slug TEXT NOT NULL UNIQUE,
            session_id TEXT NOT NULL UNIQUE, live INTEGER NOT NULL DEFAULT 1,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE session_claims (
            pubkey TEXT NOT NULL, agent_slug TEXT NOT NULL DEFAULT '',
            codename TEXT NOT NULL DEFAULT '', session_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL DEFAULT '', native_id TEXT NOT NULL DEFAULT '',
            harness TEXT NOT NULL DEFAULT '', last_active_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL, owner_backend_pubkey TEXT NOT NULL DEFAULT '',
            owner_host TEXT NOT NULL DEFAULT '', PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX idx_session_claims_expires ON session_claims(expires_at);
        CREATE INDEX idx_session_claims_session ON session_claims(session_id);
        CREATE TABLE relay_status (
            pubkey TEXT NOT NULL, session_id TEXT NOT NULL DEFAULT '', channel_h TEXT NOT NULL,
            slug TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '',
            activity TEXT NOT NULL DEFAULT '', busy INTEGER NOT NULL DEFAULT 0,
            last_seen INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0,
            expiration INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (pubkey, session_id, channel_h)
        );
        CREATE TABLE outbox (
            local_id INTEGER PRIMARY KEY AUTOINCREMENT, event_json TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending', retries INTEGER NOT NULL DEFAULT 0,
            last_error TEXT, enqueued_at INTEGER NOT NULL,
            next_attempt_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX idx_outbox_pending ON outbox(state, next_attempt_at, local_id);
        CREATE TABLE llm_calls (
            id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL,
            window_hash TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
            system_prompt TEXT NOT NULL, transcript_slice TEXT NOT NULL,
            raw_response TEXT NOT NULL, parsed_title TEXT, parsed_activity TEXT,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX idx_llm_calls_session ON llm_calls(session_id, created_at);
        CREATE INDEX idx_llm_calls_window_hash ON llm_calls(window_hash);
        CREATE TABLE trellis_commits (
            id INTEGER PRIMARY KEY AUTOINCREMENT, transaction_id INTEGER NOT NULL
        );
        CREATE TABLE trellis_replay_capsules (
            id INTEGER PRIMARY KEY AUTOINCREMENT, script_json TEXT NOT NULL
        );
        PRAGMA user_version = 4;
        "#,
    )
    .unwrap();
    seed_schema_four(&conn);
}

fn seed_schema_four(conn: &Connection) {
    conn.execute_batch(
        r#"
        INSERT INTO sessions
        VALUES ('s-old','pk1','writer','room','claude',11,'/old',1,1,10,0,0,0,'',0,0,
                'oldest','','resume-old',0,0,0);
        INSERT INTO sessions
        VALUES ('s-new','pk1','writer','room','claude',22,'/new',1,2,20,1,3,4,'topic',5,6,
                'newest','working','resume-new',1,2,7);
        INSERT INTO session_channels VALUES ('s-new','room',8);
        INSERT INTO session_aliases VALUES ('claude','harness_session','resume-new','s-new',9);
        INSERT INTO session_claims
        VALUES ('pk1','writer','code','s-new','room','resume-new','claude',10,20,'backend','host');
        INSERT INTO session_signers VALUES ('pk1','salt');
        INSERT INTO inbox VALUES ('event-in','pk1','pending','human','room','hello',11,0);
        INSERT INTO workspace_roots VALUES ('room','/work',12);
        INSERT INTO messages
        VALUES ('message','thread','room','pk1','s-new','body',13,'inbound','accepted','native',NULL);
        INSERT INTO message_recipients VALUES ('message','pk2','s-old',10);
        INSERT INTO message_recipients VALUES ('message','pk2','s-new',20);
        INSERT INTO llm_calls
            (session_id,window_hash,provider,model,system_prompt,transcript_slice,
             raw_response,parsed_title,parsed_activity,created_at)
        VALUES ('s-new','hash','provider','model','system','slice','raw','title','activity',14);
        INSERT INTO outbox (event_json,state,enqueued_at) VALUES ('{"id":"pending"}','pending',15);
        INSERT INTO outbox (event_json,state,enqueued_at) VALUES ('{"id":"published"}','published',16);
        INSERT INTO trellis_commits (transaction_id) VALUES (1);
        INSERT INTO trellis_replay_capsules (script_json) VALUES ('{}');
        "#,
    )
    .unwrap();
}

pub(super) fn create_schema_seven(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(super::super::super::ddl::SCHEMA)
        .unwrap();
    conn.execute_batch(
        r#"
        ALTER TABLE sessions DROP COLUMN work_root;
        ALTER TABLE sessions DROP COLUMN readiness_parent;
        CREATE TABLE outbox (
            local_id INTEGER PRIMARY KEY AUTOINCREMENT, event_json TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'pending', retries INTEGER NOT NULL DEFAULT 0,
            last_error TEXT, enqueued_at INTEGER NOT NULL,
            next_attempt_at INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE trellis_commits (
            id INTEGER PRIMARY KEY AUTOINCREMENT, transaction_id INTEGER NOT NULL
        );
        CREATE TABLE trellis_replay_capsules (
            id INTEGER PRIMARY KEY AUTOINCREMENT, script_json TEXT NOT NULL
        );
        PRAGMA user_version = 7;
        "#,
    )
    .unwrap();
}

pub(super) fn create_schema_eight(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(super::super::super::ddl::SCHEMA)
        .unwrap();
    conn.execute_batch(
        r#"
        ALTER TABLE sessions DROP COLUMN claimed_harness;
        ALTER TABLE sessions DROP COLUMN admitted_bundle;
        ALTER TABLE sessions DROP COLUMN admitted_transport;
        ALTER TABLE sessions DROP COLUMN endpoint_provenance;
        ALTER TABLE sessions RENAME COLUMN observed_harness TO harness;
        INSERT INTO sessions
            (pubkey, runtime_generation, harness, created_at)
        VALUES ('pk-pty', 1, 'codex', 1),
               ('pk-acp', 1, 'claude-code', 1),
               ('pk-app-server', 1, 'codex', 1);
        INSERT INTO session_locators
            (harness, locator_kind, locator_value, pubkey, created_at)
        VALUES ('codex', 'pty', 'pty-owned', 'pk-pty', 1),
               ('claude-code', 'acp', 'acp-foreign', 'pk-pty', 2),
               ('claude-code', 'acp', 'acp-owned', 'pk-acp', 1),
               ('codex', 'acp', 'app-server-owned', 'pk-app-server', 1);
        PRAGMA user_version = 8;
        "#,
    )
    .unwrap();
}

pub(super) fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}
