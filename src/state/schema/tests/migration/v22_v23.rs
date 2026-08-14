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
                bundle: "codex-app".into(),
                transport: "app-server".into(),
                endpoint_provenance: "launch".into(),
            },
        )
        .unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "user_version", 22).unwrap();
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
                bundle: "pi-rpc".into(),
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
    assert_eq!(version(&conn), 23);
}
