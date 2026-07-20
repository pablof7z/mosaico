use super::*;

struct PtyCleanup(String);

impl Drop for PtyCleanup {
    fn drop(&mut self) {
        let _ = mosaico::pty::kill(&self.0);
    }
}

#[test]
fn native_id_adopts_once_then_attaches_to_the_same_pty() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    std::fs::write(
        home.dir.path().join("config.json"),
        serde_json::json!({
            "whitelistedPubkeys": [],
            "backendName": "test-host",
            "relays": [shared_relay_url()],
            "indexerRelay": shared_relay_url(),
            "mosaicoPrivateKey": "b53809614e9c41b923dd5546e438e48e9bcbee04b9c7c50bae0b085954e03422"
        })
        .to_string(),
    )
    .unwrap();
    let root = unique_session("native-resume");
    let work_dir = home.dir.path().join(&root);
    add_workspace_mapping(&home, &root, &work_dir);
    std::fs::write(
        home.dir.path().join("harnesses.json"),
        r#"{"opencode-pty":{"harness":"opencode","transport":"pty","args":["forever"]}}"#,
    )
    .unwrap();

    let native_id = "native-id-without-uuid-shape";
    let data_home = home.dir.path().join("data");
    let database = data_home.join("opencode/opencode.db");
    std::fs::create_dir_all(database.parent().unwrap()).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute(
            "CREATE TABLE session (id TEXT PRIMARY KEY, directory TEXT NOT NULL)",
            [],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session (id, directory) VALUES (?1, ?2)",
            [native_id, work_dir.to_str().unwrap()],
        )
        .unwrap();
    drop(connection);

    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    let data_home = data_home.to_string_lossy().into_owned();
    let env = [
        ("HOME", isolated_home.as_str()),
        ("XDG_DATA_HOME", data_home.as_str()),
    ];
    let adopted = run_cli_with_env_in_dir(&home, &["resume", native_id], &env, &work_dir);
    assert!(
        adopted.status.success(),
        "native adoption failed: {}",
        String::from_utf8_lossy(&adopted.stderr)
    );
    assert!(
        String::from_utf8_lossy(&adopted.stderr).contains("Adopted and resumed @"),
        "unexpected adoption output: {}",
        String::from_utf8_lossy(&adopted.stderr)
    );

    let store = Store::open(&home.store_path()).unwrap();
    let pubkey = store
        .resolve_pubkey_by_locator("opencode", "native_resume", native_id)
        .unwrap()
        .expect("native locator owner");
    let session = store.get_session(&pubkey).unwrap().unwrap();
    assert_eq!(session.agent_slug, "opencode");
    assert_eq!(session.channel_h, root);
    let metadata = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.agent == "opencode" && metadata.root == root)
        .expect("adopted PTY metadata");
    assert_eq!(
        metadata.command,
        ["opencode", "forever", "--session", native_id]
    );
    let cleanup = PtyCleanup(metadata.id.clone());

    let attached = run_cli_with_env_in_dir(&home, &["resume", native_id], &env, &work_dir);
    assert!(attached.status.success());
    assert!(
        String::from_utf8_lossy(&attached.stderr).contains("Attached to @"),
        "unexpected attach output: {}",
        String::from_utf8_lossy(&attached.stderr)
    );
    let sessions = Store::open(&home.store_path())
        .unwrap()
        .list_running_sessions()
        .unwrap();
    assert_eq!(
        sessions
            .iter()
            .filter(|session| session.pubkey == pubkey)
            .count(),
        1
    );

    drop(cleanup);
    stop_daemon(&home);
}
