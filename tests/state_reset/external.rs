use super::support::*;

#[test]
fn explicit_home_reset_preserves_external_config_and_clears_its_attachment_state() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = fixture.path().join("selected-runtime");
    let config = fixture.path().join("operator/config.json");
    let attachments = fixture.path().join("received-attachments");
    let kept = seed_configuration_at(fixture.path(), &home, &config, &attachments);
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    write(&attachments.join("nested/received.bin"), b"attachment");

    let reset = command_with_paths(
        fixture.path(),
        &home,
        &config,
        &["daemon", "reset-state", CONFIRM],
    );
    assert!(
        reset.status.success(),
        "external-config reset failed: {}",
        String::from_utf8_lossy(&reset.stderr)
    );
    assert!(!home.join("state.db").exists());
    assert!(!home.join("nmp.redb").exists());
    assert!(!home.join("sessions").exists());
    assert!(attachments.is_dir());
    assert!(std::fs::read_dir(&attachments).unwrap().next().is_none());
    for (path, bytes) in kept {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "configuration changed: {}",
            path.display()
        );
    }
}

#[test]
fn attachment_target_overlapping_external_config_refuses_before_mutation() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = fixture.path().join("selected-runtime");
    let config_directory = fixture.path().join("operator");
    let config = config_directory.join("config.json");
    let kept = seed_configuration_at(fixture.path(), &home, &config, &config_directory);
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    let before_state = std::fs::read(home.join("state.db")).unwrap();
    let before_nmp = std::fs::read(home.join("nmp.redb")).unwrap();

    let reset = command_with_paths(
        fixture.path(),
        &home,
        &config,
        &["daemon", "reset-state", CONFIRM],
    );
    assert!(!reset.status.success(), "config overlap must refuse reset");
    assert_eq!(std::fs::read(home.join("state.db")).unwrap(), before_state);
    assert_eq!(std::fs::read(home.join("nmp.redb")).unwrap(), before_nmp);
    assert!(!home.join("daemon.inhibit").exists());
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

fn assert_unchanged(before: Vec<(std::path::PathBuf, Vec<u8>)>) {
    for (path, bytes) in before {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "broad-root preflight mutated {}",
            path.display()
        );
    }
}

fn runtime_snapshot(home: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    [
        "state.db",
        "nmp.redb",
        "sessions/one/hook-calls.jsonl",
        "tmp/attachments/received.bin",
        "logs/group-mgmt.log",
    ]
    .map(|name| home.join(name))
    .into_iter()
    .map(|path| {
        let bytes = std::fs::read(&path).unwrap();
        (path, bytes)
    })
    .collect()
}

#[test]
fn home_directory_cannot_be_selected_as_mosaico_runtime_root() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = fixture.path();
    let config = home.join("operator/config.json");
    let kept = seed_configuration_at(home, home, &config, &home.join("received"));
    seed_runtime(home);
    superseded_epoch_store(&home.join("nmp.redb"));
    let runtime = runtime_snapshot(home);

    let reset = command_with_paths(home, home, &config, &["daemon", "reset-state", CONFIRM]);

    assert!(!reset.status.success(), "HOME must refuse as runtime root");
    assert!(!home.join("daemon.inhibit").exists());
    assert_unchanged(runtime);
    assert_unchanged(kept);
}

#[test]
fn temp_directory_cannot_be_selected_as_mosaico_runtime_root() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let operator_home = fixture.path().join("operator-home");
    let temp_root = fixture.path().join("temp-root");
    let config = temp_root.join("config.json");
    let kept = seed_configuration_at(
        &operator_home,
        &temp_root,
        &config,
        &temp_root.join("received"),
    );
    seed_runtime(&temp_root);
    superseded_epoch_store(&temp_root.join("nmp.redb"));
    let runtime = runtime_snapshot(&temp_root);

    let reset = command_with_paths_and_temp(
        &operator_home,
        &temp_root,
        &config,
        &temp_root,
        &["daemon", "reset-state", CONFIRM],
    );

    assert!(
        !reset.status.success(),
        "temp root must refuse as runtime root"
    );
    assert!(!temp_root.join("daemon.inhibit").exists());
    assert_unchanged(runtime);
    assert_unchanged(kept);
}
