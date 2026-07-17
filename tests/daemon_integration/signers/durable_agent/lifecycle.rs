use super::*;

fn launch(home: &Home, slug: &str, mode: &str) -> std::process::Output {
    configure_pty_agent(home, slug, mode);
    run_cli(home, &["agents", slug, "--workspace", "tmp"])
}

pub(super) async fn assert_supervisor_releases_reservations(home: &Home, slug: &str) {
    let running = launch(home, slug, "sleep-2");
    assert!(
        running.status.success(),
        "{}",
        String::from_utf8_lossy(&running.stderr)
    );
    let conflict = launch(home, slug, "exit-0");
    assert!(
        !conflict.status.success(),
        "reservation must remain exclusive while the no-hook child is alive"
    );

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let after_exit = launch(home, slug, "exit-0");
    assert!(
        after_exit.status.success(),
        "normal child exit did not release reservation: {}",
        String::from_utf8_lossy(&after_exit.stderr)
    );

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let failed = launch(home, slug, "exit-1");
    assert!(
        failed.status.success(),
        "supervisor process failed to start the failing harness: {}",
        String::from_utf8_lossy(&failed.stderr)
    );
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let after_failure = launch(home, slug, "exit-0");
    assert!(
        after_failure.status.success(),
        "failed harness did not release reservation: {}",
        String::from_utf8_lossy(&after_failure.stderr)
    );
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}
