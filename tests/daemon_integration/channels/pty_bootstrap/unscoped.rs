use super::*;

#[test]
fn direct_launch_from_unknown_directory_starts_unscoped_in_that_directory() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new();
    write_config(&home, true);
    std::fs::write(home.dir.path().join("workspaces.json"), "{}").unwrap();

    let work_dir = home.dir.path().join("loose-files");
    std::fs::create_dir_all(&work_dir).unwrap();
    let agent = "unscoped-launch-agent";
    configure_pty_agent(&home, agent, "forever");

    let output = run_cli_with_env_in_dir(&home, &[agent], &[], &work_dir);
    assert!(
        output.status.success(),
        "unscoped launch failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let session = wait_for_alive(&home, agent, "");
    assert_eq!(session.work_root, "");
    let store = Store::open(&home.store_path()).unwrap();
    assert!(store
        .list_session_routes(&session.pubkey)
        .unwrap()
        .is_empty());
    assert!(store
        .list_workspace_bindings()
        .unwrap()
        .iter()
        .all(|binding| !binding.channel_h.is_empty()));

    let metadata = mosaico::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.agent == agent)
        .expect("unscoped PTY metadata");
    assert_eq!(metadata.root, "");
    assert_eq!(
        std::path::Path::new(&metadata.cwd).canonicalize().unwrap(),
        work_dir.canonicalize().unwrap(),
        "the harness must retain the caller's cwd"
    );

    let _ = mosaico::pty::kill(&metadata.id);
    stop_daemon(&home);
}
