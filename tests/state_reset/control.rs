use super::support::*;

#[test]
fn symlinked_inhibitor_refuses_before_truncating_config_or_runtime() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    std::os::unix::fs::symlink(home.join("config.json"), home.join("daemon.inhibit")).unwrap();
    let runtime = [
        home.join("state.db"),
        home.join("nmp.redb"),
        home.join("sessions/one/hook-calls.jsonl"),
        home.join("tmp/attachments/received.bin"),
        home.join("daemon.log"),
    ]
    .map(|path| {
        let bytes = std::fs::read(&path).unwrap();
        (path, bytes)
    });

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);

    assert!(
        !reset.status.success(),
        "symlinked inhibitor must refuse reset"
    );
    assert!(
        std::fs::symlink_metadata(home.join("daemon.inhibit"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "preflight must not mutate the inhibitor"
    );
    for (path, bytes) in runtime.into_iter().chain(kept) {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "preflight mutated {}",
            path.display()
        );
    }
}

#[test]
fn existing_inhibitor_hard_linked_to_config_is_never_truncated() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    std::fs::hard_link(home.join("config.json"), home.join("daemon.inhibit")).unwrap();
    let config = std::fs::read(home.join("config.json")).unwrap();

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);

    assert!(
        reset.status.success(),
        "existing regular inhibitor should be left alone: {}",
        String::from_utf8_lossy(&reset.stderr)
    );
    assert_eq!(std::fs::read(home.join("config.json")).unwrap(), config);
    assert_eq!(std::fs::read(home.join("daemon.inhibit")).unwrap(), config);
    assert!(!home.join("state.db").exists());
    assert!(!home.join("nmp.redb").exists());
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
