use super::*;

#[test]
fn schema_twenty_one_drops_unattributed_relay_caches_but_keeps_local_intent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    drop(Store::open(&path).expect("fresh schema opens"));

    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "DROP TABLE relay_projection_rows;
         DROP TABLE relay_projection_owners;
         DROP TABLE relay_projection_observations;
         INSERT INTO relay_channels
             (channel_h, name, about, parent, created_at, updated_at)
             VALUES ('phantom', 'phantom', '', '', 1, 1);
         INSERT INTO relay_profiles
             (pubkey, name, slug, agent_slug, host, is_backend,
              agents_json, workspaces_json, updated_at)
             VALUES ('peer', 'peer', 'peer', '', '', 0, '[]', '[]', 1);
         INSERT INTO channel_resolution_intents
             (parent, name, channel_h, created_at)
             VALUES ('root', 'wanted', 'reserved', 1);
         PRAGMA user_version = 21;",
    )
    .unwrap();
    drop(conn);

    drop(Store::open(&path).expect("schema twenty one upgrades to current"));
    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 22);
    assert_eq!(count(&conn, "relay_channels"), 0);
    assert_eq!(count(&conn, "relay_profiles"), 0);
    assert_eq!(count(&conn, "relay_projection_owners"), 0);
    assert_eq!(count(&conn, "relay_projection_rows"), 0);
    assert_eq!(count(&conn, "channel_resolution_intents"), 1);
}
