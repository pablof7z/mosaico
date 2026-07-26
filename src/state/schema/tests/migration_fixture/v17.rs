use super::*;

pub(in crate::state::schema::tests) fn downgrade_channel_context_to_v17(conn: &Connection) {
    conn.execute_batch(
        r#"
        DROP INDEX idx_sessions_runtime;
        ALTER TABLE sessions
            ADD COLUMN channel_h TEXT NOT NULL DEFAULT '';
        CREATE INDEX idx_sessions_runtime
            ON sessions(runtime_state, channel_h);

        DROP INDEX idx_session_channels_channel;
        ALTER TABLE session_channels RENAME TO migration_fixture_session_channels;
        CREATE TABLE session_channels (
            pubkey TEXT NOT NULL,
            channel_h TEXT NOT NULL,
            granted_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        INSERT INTO session_channels (pubkey, channel_h, granted_at)
        SELECT pubkey, channel_h, joined_at
          FROM migration_fixture_session_channels;
        DROP TABLE migration_fixture_session_channels;
        CREATE INDEX idx_session_channels_channel
            ON session_channels(channel_h, pubkey);

        DROP INDEX idx_session_standing_state;
        ALTER TABLE session_standing RENAME TO migration_fixture_session_standing;
        CREATE TABLE session_standing (
            pubkey TEXT NOT NULL,
            channel_h TEXT NOT NULL,
            state TEXT NOT NULL CHECK (state IN ('member', 'retained', 'absent')),
            retain_until INTEGER NOT NULL DEFAULT 0,
            standing_epoch INTEGER NOT NULL DEFAULT 1,
            session_lifecycle_epoch INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (pubkey, channel_h)
        );
        INSERT INTO session_standing
            (pubkey, channel_h, state, retain_until, standing_epoch,
             session_lifecycle_epoch, updated_at)
        SELECT pubkey, channel_h, state, 0, standing_epoch,
               session_lifecycle_epoch, updated_at
          FROM migration_fixture_session_standing;
        DROP TABLE migration_fixture_session_standing;
        CREATE INDEX idx_session_standing_due
            ON session_standing(state, retain_until);

        DROP INDEX idx_relay_channels_named_sibling;
        ALTER TABLE relay_channels RENAME TO migration_fixture_relay_channels;
        CREATE TABLE relay_channels (
            channel_h TEXT PRIMARY KEY,
            name TEXT NOT NULL DEFAULT '',
            about TEXT NOT NULL DEFAULT '',
            parent TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(parent, name)
        );
        INSERT INTO relay_channels
        SELECT * FROM migration_fixture_relay_channels;
        DROP TABLE migration_fixture_relay_channels;

        ALTER TABLE relay_status DROP COLUMN workspace;
        ALTER TABLE relay_status DROP COLUMN branch;
        DROP TABLE relay_status_sets;
        "#,
    )
    .unwrap();
}
