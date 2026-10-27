use super::*;

/// Launch from a terminal: these reservation checks exercise the interactive
/// PTY supervisor's startup and exit reporting.
fn launch(home: &Home, slug: &str, mode: &str) -> TerminalRun {
    configure_pty_agent(home, slug, mode);
    let isolated_home = home.dir.path().to_string_lossy().into_owned();
    run_cli_in_terminal(
        home,
        &[slug, "--", "--fresh"],
        &[("HOME", &isolated_home)],
        &home.dir.path().join("work"),
    )
}

pub(super) async fn assert_supervisor_releases_reservations(home: &Home, slug: &str) {
    let release = home.dir.path().join("release-harness");
    let running = launch(home, slug, "wait-file");
    assert!(running.handed_off, "{}", running.output);
    let conflict = launch(home, slug, "exit-0");
    assert!(
        conflict.failed(),
        "reservation must remain exclusive while the no-hook child is alive: {}",
        conflict.output
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
        after_exit.failed(),
        "an already-exited child must not be reported as launched: {}",
        after_exit.output
    );
    assert!(
        after_exit.output.contains("exited during startup"),
        "{}",
        after_exit.output
    );
    let after_clean_failure = launch(home, slug, "sleep-2");
    assert!(
        after_clean_failure.handed_off,
        "clean startup failure retained the reservation: {}",
        after_clean_failure.output
    );

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let failed = launch(home, slug, "exit-1");
    assert!(
        failed.failed(),
        "a failing harness must not be reported as launched: {}",
        failed.output
    );
    let after_failure = launch(home, slug, "sleep-2");
    assert!(
        after_failure.handed_off,
        "failed harness did not release reservation: {}",
        after_failure.output
    );
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
}
