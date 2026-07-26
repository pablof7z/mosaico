use super::*;

fn launch(home: &Home, slug: &str, mode: &str) -> std::process::Output {
    configure_pty_agent(home, slug, mode);
    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    run_cli_with_env_in_dir(
        home,
        &[slug, "--", "--fresh"],
        &[("HOME", &isolated_home)],
        &home.dir.path().join("work"),
    )
}

pub(super) async fn assert_supervisor_releases_reservations(home: &Home, slug: &str) {
    let release = home.dir.path().join("release-harness");
    let running = launch(home, slug, "wait-file");
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

    std::fs::write(&release, "").unwrap();
    assert!(wait_until(std::time::Duration::from_secs(10), || {
        Store::open(&home.store_path())
            .map(|store| {
                store
                    .list_running_sessions()
                    .unwrap_or_default()
                    .iter()
                    .all(|session| session.agent_slug != slug)
            })
            .unwrap_or(false)
    }));
    std::fs::remove_file(&release).unwrap();
    let after_exit = launch(home, slug, "exit-0");
    assert!(
        !after_exit.status.success(),
        "an already-exited child must not be reported as launched"
    );
    assert!(
        String::from_utf8_lossy(&after_exit.stderr).contains("exited during startup"),
        "{}",
        String::from_utf8_lossy(&after_exit.stderr)
    );
    let after_clean_failure = launch(home, slug, "sleep-2");
    assert!(
        after_clean_failure.status.success(),
        "clean startup failure retained the reservation: {}",
        String::from_utf8_lossy(&after_clean_failure.stderr)
    );

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let failed = launch(home, slug, "exit-1");
    assert!(
        !failed.status.success(),
        "a failing harness must not be reported as launched: {}",
        String::from_utf8_lossy(&failed.stderr)
    );
    let after_failure = launch(home, slug, "sleep-2");
    assert!(
        after_failure.status.success(),
        "failed harness did not release reservation: {}",
        String::from_utf8_lossy(&after_failure.stderr)
    );
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}
