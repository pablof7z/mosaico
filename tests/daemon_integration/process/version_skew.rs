use super::*;

#[test]
fn version_skew_client_detects_and_respawns() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let home = Home::new().with_backend_key();

    let old = run_cli_proto(&home, &["who", "--all-workspaces"], "1");
    assert!(
        old.status.success(),
        "proto-1 who failed: {}",
        String::from_utf8_lossy(&old.stderr)
    );
    assert!(home.sock().exists(), "daemon should be up at proto 1");

    let current = run_cli_proto(&home, &["who", "--all-workspaces"], "2");
    assert!(
        current.status.success(),
        "proto-2 client failed to respawn+succeed: {}",
        String::from_utf8_lossy(&current.stderr)
    );
    stop_daemon(&home);
}

fn run_cli_proto(home: &Home, args: &[&str], protocol: &str) -> std::process::Output {
    let mut command = std::process::Command::new(bin());
    command
        .args(args)
        .env_remove("MOSAICO_AGENT")
        .env_remove("MOSAICO_PUBKEY")
        .env_remove("MOSAICO_PTY_SESSION")
        .env_remove("MOSAICO_PTY_SOCKET")
        .env_remove("MOSAICO_CHANNEL")
        .env_remove("MOSAICO_EPHEMERAL")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_BIN", bin())
        .env("MOSAICO_DAEMON_GRACE_S", "30")
        .env("MOSAICO_PROTOCOL", protocol);
    command.output().expect("run mosaico")
}
