use super::*;

#[test]
fn endpoint_socket_comes_from_launch_metadata() {
    let meta = LaunchMetadata {
        id: "pty-1".into(),
        socket: "/tmp/pty-1.sock".into(),
        supervisor_pid: 42,
        instance_token: "token-1".into(),
        child_pid: None,
        agent: "agent".into(),
        root: "/tmp".into(),
        cwd: "/tmp".into(),
        ephemeral: false,
        command: vec!["codex".into()],
    };

    assert_eq!(
        endpoint_socket_in("pty-1", [meta]),
        Some("/tmp/pty-1.sock".into())
    );
    assert_eq!(endpoint_socket_in("acp-1", std::iter::empty()), None);
}

#[cfg(unix)]
#[test]
fn socket_path_stays_short_for_long_mosaico_home() {
    use std::os::unix::ffi::OsStrExt;

    let mosaico_home = std::path::Path::new(
        "/var/folders/kx/13lj0yd976x0tn90z1ntqbn80000gn/T/mosaico-e2e/mosaico-b/mosaico",
    );
    let path = socket_dir_for(mosaico_home, 501).join("testing-lead-1783399436-28334.sock");

    assert!(path.as_os_str().as_bytes().len() < 100);
}

#[test]
fn ownership_requires_exact_endpoint_and_instance_token_arguments() {
    let command =
        "/opt/mosaico __pty-supervisor --id grok-123-456 --instance-token token-2 -- echo";
    assert!(command_owns_endpoint(command, "grok-123-456", "token-2"));
    assert!(!command_owns_endpoint(command, "grok-123-45", "token-2"));
    assert!(!command_owns_endpoint(command, "grok-123-456", "token"));
    assert!(!command_owns_endpoint(
        "/opt/mosaico unrelated -- __pty-supervisor --id grok-123-456 --instance-token token-2",
        "grok-123-456",
        "token-2"
    ));
    assert!(!command_owns_endpoint(
        "/opt/mosaico __pty-supervisor --id other --instance-token other -- --id grok-123-456 --instance-token token-2",
        "grok-123-456",
        "token-2"
    ));
}

#[cfg(unix)]
#[test]
fn owned_child_fallback_escalates_past_ignored_hup() {
    use portable_pty::{native_pty_system, CommandBuilder, PtySize};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "trap '' HUP; exec /bin/sleep 60"]);
    let mut child = pair.slave.spawn_command(command).unwrap();
    let pid = i32::try_from(child.process_id().unwrap()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    terminate_owned_child(pid).unwrap();

    assert!(child.try_wait().unwrap().is_some());
}
