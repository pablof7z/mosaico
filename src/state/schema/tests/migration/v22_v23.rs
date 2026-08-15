use super::*;

#[test]
fn schema_twenty_two_admits_pi_rpc_without_losing_runtime_rows() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("state.db");
    let store = Store::open(&path).unwrap();
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: "existing".into(),
                observed_harness: "codex".into(),
                agent_slug: "codex".into(),
                launch_channel_h: "root".into(),
                work_root: "root".into(),
                child_pid: None,
                now: 1,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: "codex".into(),
                claimed_harness: String::new(),
                preset: "lab".into(),
                transport: "app-server".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "ALTER TABLE sessions DROP COLUMN admitted_preset;
         ALTER TABLE sessions ADD COLUMN admitted_bundle TEXT NOT NULL DEFAULT '';
         PRAGMA user_version = 22;",
    )
    .unwrap();
    drop(conn);

    let store = Store::open(&path).unwrap();
    assert!(store.get_session("existing").unwrap().is_some());
    store
        .reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: "pi".into(),
                observed_harness: "pi".into(),
                agent_slug: "pi".into(),
                launch_channel_h: "root".into(),
                work_root: "root".into(),
                child_pid: None,
                now: 2,
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: "pi".into(),
                claimed_harness: String::new(),
                preset: String::new(),
                transport: "pi-rpc".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    store
        .put_session_locator("pi", crate::state::LOCATOR_PI_RPC, "endpoint", "pi", 2)
        .unwrap();
    drop(store);

    let conn = Connection::open(path).unwrap();
    assert_eq!(version(&conn), 24);
}
