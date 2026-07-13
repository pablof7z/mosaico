use super::*;

#[test]
fn channel_list_from_registered_workspace_sends_channel_param() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = Home::new();

    let out = run_cli_with_env_in_dir(
        &home,
        &["channel", "list"],
        &[],
        std::path::Path::new("/tmp"),
    );
    assert!(
        out.status.success(),
        "channel list should succeed; stdout={}; stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.lines().any(|line| line.contains("tmp")),
        "channel list should render the resolved root; stdout={stdout}"
    );

    stop_daemon(&home);
}
