use rusqlite::Connection;

/// Restore the NIP-29 cache shape present through schema 21.
///
/// Migration tests construct an older schema by reversing changes from the
/// current DDL, where these tables are intentionally absent.
pub(in crate::state::schema::tests) fn restore_relay_group_tables(conn: &Connection) {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS relay_channels (
            channel_h TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
            about TEXT NOT NULL DEFAULT '', parent TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_relay_channels_named_sibling
            ON relay_channels(parent, name) WHERE parent<>'' AND name<>'';
        CREATE TABLE IF NOT EXISTS relay_channel_members (
            channel_h TEXT NOT NULL, pubkey TEXT NOT NULL,
            role TEXT NOT NULL DEFAULT 'member', updated_at INTEGER NOT NULL,
            PRIMARY KEY (channel_h, pubkey)
        );
        CREATE INDEX IF NOT EXISTS idx_relay_channel_members_pubkey
            ON relay_channel_members(pubkey, role);
        CREATE TABLE IF NOT EXISTS relay_channel_member_sets (
            channel_h TEXT NOT NULL, role TEXT NOT NULL, updated_at INTEGER NOT NULL,
            PRIMARY KEY (channel_h, role)
        );
        "#,
    )
    .unwrap();
}

/// Restore every relay-derived table present in schema 21.
pub(in crate::state::schema::tests) fn restore_relay_derived_tables(conn: &Connection) {
    super::downgrade_launch_admission_to_v22(conn);
    restore_relay_group_tables(conn);
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS nmp_event_arrivals;
        DROP TABLE IF EXISTS message_attachments;
        CREATE TABLE IF NOT EXISTS relay_profiles (
            pubkey TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '',
            slug TEXT NOT NULL DEFAULT '', agent_slug TEXT NOT NULL DEFAULT '',
            host TEXT NOT NULL DEFAULT '', is_backend INTEGER NOT NULL DEFAULT 0,
            agents_json TEXT NOT NULL DEFAULT '[]',
            workspaces_json TEXT NOT NULL DEFAULT '[]', updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS relay_status (
            pubkey TEXT NOT NULL, channel_h TEXT NOT NULL,
            slug TEXT NOT NULL DEFAULT '', title TEXT NOT NULL DEFAULT '',
            activity TEXT NOT NULL DEFAULT '', workspace TEXT NOT NULL DEFAULT '',
            branch TEXT NOT NULL DEFAULT '', state TEXT NOT NULL,
            state_since INTEGER NOT NULL DEFAULT 0, last_seen INTEGER NOT NULL DEFAULT 0,
            updated_at INTEGER NOT NULL DEFAULT 0, expiration INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (pubkey, channel_h)
        );
        CREATE INDEX IF NOT EXISTS idx_relay_status_channel
            ON relay_status(channel_h, expiration);
        CREATE TABLE IF NOT EXISTS relay_status_sets (
            pubkey TEXT PRIMARY KEY, updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS relay_events (
            id TEXT PRIMARY KEY, kind INTEGER NOT NULL, pubkey TEXT NOT NULL,
            created_at INTEGER NOT NULL, channel_h TEXT NOT NULL DEFAULT '',
            d_tag TEXT NOT NULL DEFAULT '', content TEXT NOT NULL DEFAULT '',
            tags_json TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_relay_events_channel
            ON relay_events(channel_h, created_at, id);
        CREATE INDEX IF NOT EXISTS idx_relay_events_kind
            ON relay_events(kind);
        CREATE INDEX IF NOT EXISTS idx_relay_events_addr
            ON relay_events(kind, pubkey, d_tag);
        CREATE TABLE IF NOT EXISTS relay_reactions (
            reaction_id TEXT PRIMARY KEY, target_message_id TEXT NOT NULL,
            channel_h TEXT NOT NULL DEFAULT '', reactor_pubkey TEXT NOT NULL,
            emoji TEXT NOT NULL DEFAULT '+', created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_relay_reactions_target
            ON relay_reactions(target_message_id, created_at);
        "#,
    )
    .unwrap();
    restore_relay_message_tables(conn);
}

/// Restore only the message cache shape used through schema 21.
///
/// Older fixture downgrades call this after removing schema-18 group/status
/// shapes, so it must not recreate any unrelated relay table.
pub(in crate::state::schema::tests) fn restore_relay_message_tables(conn: &Connection) {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS messages (
            message_id TEXT PRIMARY KEY, thread_id TEXT NOT NULL DEFAULT '',
            channel_h TEXT NOT NULL, author_pubkey TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL,
            sync_state TEXT NOT NULL DEFAULT 'accepted', native_event_id TEXT,
            error TEXT, attachment_dir TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_messages_channel
            ON messages(channel_h, created_at, message_id);
        CREATE INDEX IF NOT EXISTS idx_messages_native
            ON messages(native_event_id);
        CREATE INDEX IF NOT EXISTS idx_messages_author_pubkey
            ON messages(author_pubkey, sync_state, created_at);
        CREATE TABLE IF NOT EXISTS message_recipients (
            message_id TEXT NOT NULL, recipient_pubkey TEXT NOT NULL,
            delivered_at INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (message_id, recipient_pubkey)
        );
        CREATE INDEX IF NOT EXISTS idx_message_recipients_pubkey
            ON message_recipients(recipient_pubkey, delivered_at);
        "#,
    )
    .unwrap();
}
