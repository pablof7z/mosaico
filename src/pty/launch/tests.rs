use super::*;
use std::process::Command;

#[test]
fn endpoint_ids_are_unique_within_the_same_process_and_second() {
    let first = new_endpoint_id("grok");
    let second = new_endpoint_id("grok");
    assert_ne!(first, second);
}

#[cfg(unix)]
#[test]
fn generated_endpoint_stays_within_unix_socket_limits() {
    use std::os::unix::ffi::OsStrExt;

    let id = new_endpoint_id(&"very-long-agent-name-".repeat(8));
    assert!(session_socket(&id).as_os_str().as_bytes().len() < 100);
}

#[test]
fn supervisor_exit_before_readiness_is_a_launch_error() {
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("supervisor.log");
    std::fs::write(&log_path, "provider executable was not found\n").unwrap();
    let mut child = Command::new("/bin/sh")
        .args(["-c", "exit 127"])
        .spawn()
        .unwrap();
    let id = new_endpoint_id("missing-provider");
    let meta = LaunchMetadata {
        id,
        socket: temp.path().join("missing.sock").to_string_lossy().into(),
        supervisor_pid: child.id(),
        instance_token: "test-token".into(),
        child_pid: None,
        agent: "missing-provider".into(),
        root: "test".into(),
        cwd: temp.path().to_string_lossy().into(),
        ephemeral: false,
        command: vec!["missing-provider".into()],
    };

    let error = wait_until_ready(&mut child, &meta, &log_path)
        .unwrap_err()
        .to_string();

    assert!(error.contains("exited during startup"), "{error}");
    assert!(
        error.contains("provider executable was not found"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn metadata_write_failure_rolls_back_the_spawned_supervisor() {
    use crate::test_env::EnvGuard;

    let home = tempfile::tempdir().unwrap();
    let _env = EnvGuard::set("MOSAICO_HOME", home.path());
    let id = new_endpoint_id("metadata-failure");
    std::fs::create_dir_all(
        crate::config::mosaico_home()
            .join("pty")
            .join(format!("{id}.json")),
    )
    .unwrap();
    let args = SpawnSessionArgs {
        id: Some(id.clone()),
        agent: "codex".into(),
        root: "test".into(),
        cwd: home.path().to_path_buf(),
        channel: None,
        session_name: None,
        ephemeral: false,
        command: vec!["sleep".into(), "60".into()],
        env: vec![],
        env_remove: vec![],
    };

    let error = spawn_session_with_executable(args, "/bin/sleep").unwrap_err();

    assert!(format!("{error:#}").contains("writing pty metadata"));
    let processes = Command::new("ps")
        .args(["-ax", "-o", "command="])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&processes.stdout).contains(&id));
}
