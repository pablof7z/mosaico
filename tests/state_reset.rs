//! Real-binary contract for destructive, config-preserving selected-state reset.

#[path = "state_reset/support.rs"]
mod support;
use support::*;

#[path = "state_reset/process.rs"]
mod process;

#[path = "state_reset/external.rs"]
mod external;

#[path = "state_reset/control.rs"]
mod control;

#[test]
fn incompatible_store_offers_one_full_reset_that_preserves_configuration() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    let sibling = fixture.path().join(".mosaico-instances/relay2/state.db");
    write(&sibling, b"other instance");

    let startup = command(fixture.path(), &["daemon"]);
    assert!(
        !startup.status.success(),
        "retired NMP epoch must refuse startup"
    );
    let startup_evidence = format!(
        "{}\n{}",
        String::from_utf8_lossy(&startup.stderr),
        std::fs::read_to_string(home.join("daemon.log")).unwrap_or_default()
    );
    assert!(
        startup_evidence.contains(&format!("mosaico daemon reset-state {CONFIRM}")),
        "startup must offer the exact coherent recovery: {startup_evidence}"
    );
    // Add sidecar sentinels only after startup has proved the incompatible NMP
    // branch. Arbitrary sidecar bytes before that assertion could make SQLite
    // fail first and turn this into a false-positive startup test.
    write(&home.join("state.db-wal"), b"runtime");
    write(&home.join("state.db-shm"), b"runtime");
    write(&home.join("state.db-journal"), b"runtime");
    let stale_socket_dir = pty_socket_directory(&home);
    write(&stale_socket_dir.join("already-dead.sock"), b"stale");

    let unconfirmed = command(fixture.path(), &["daemon", "reset-state"]);
    assert!(
        !unconfirmed.status.success(),
        "reset must require confirmation"
    );
    assert!(home.join("state.db").exists());
    assert!(home.join("nmp.redb").exists());

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        reset.status.success(),
        "confirmed reset failed: {}",
        String::from_utf8_lossy(&reset.stderr)
    );

    for path in [
        "state.db",
        "state.db-wal",
        "state.db-shm",
        "state.db-journal",
        "nmp.redb",
        "daemon.sock",
        "daemon.log",
        "sessions",
        "pty",
        "tmp",
        "harness-profiles",
        "harness-context",
        "relay-assist",
        "logs",
    ] {
        assert!(!home.join(path).exists(), "runtime survived reset: {path}");
    }
    for (path, bytes) in kept {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "config changed: {}",
            path.display()
        );
    }
    assert_eq!(std::fs::read(sibling).unwrap(), b"other instance");
    assert!(
        home.join("daemon.lock").exists(),
        "lock inode must remain stable"
    );
    assert!(
        home.join("daemon.inhibit").exists(),
        "hooks stay inhibited until restart"
    );
    assert!(
        !stale_socket_dir.exists(),
        "external stale PTY sockets survived reset"
    );
}

#[test]
fn reset_clears_a_configured_external_attachment_directory_without_changing_config() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let attachments = fixture.path().join("received-attachments");
    let kept = seed_configuration(fixture.path(), &home, &attachments);
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    write(&attachments.join("nested/received.bin"), b"attachment");

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        reset.status.success(),
        "{}",
        String::from_utf8_lossy(&reset.stderr)
    );
    assert!(
        attachments.is_dir(),
        "configured receive directory itself must remain"
    );
    assert!(
        std::fs::read_dir(&attachments).unwrap().next().is_none(),
        "configured external attachment contents survived reset"
    );
    for (path, bytes) in kept {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "config changed: {}",
            path.display()
        );
    }
}

#[test]
fn attachment_directory_inside_a_sibling_instance_refuses_without_touching_either() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let sibling = fixture.path().join(".mosaico-instances/relay2");
    let sibling_attachments = sibling.join("received-attachments");
    let kept = seed_configuration(fixture.path(), &home, &sibling_attachments);
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    write(&sibling.join("config.json"), b"sibling config");
    write(&sibling.join("state.db"), b"sibling state");
    write(
        &sibling_attachments.join("received.bin"),
        b"sibling attachment",
    );
    let selected_state = std::fs::read(home.join("state.db")).unwrap();
    let selected_nmp = std::fs::read(home.join("nmp.redb")).unwrap();

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        !reset.status.success(),
        "cross-instance target must refuse reset"
    );
    assert_eq!(
        std::fs::read(home.join("state.db")).unwrap(),
        selected_state
    );
    assert_eq!(std::fs::read(home.join("nmp.redb")).unwrap(), selected_nmp);
    assert_eq!(
        std::fs::read(sibling.join("state.db")).unwrap(),
        b"sibling state"
    );
    assert_eq!(
        std::fs::read(sibling_attachments.join("received.bin")).unwrap(),
        b"sibling attachment"
    );
    assert!(!home.join("daemon.inhibit").exists());
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn attachment_overlap_refuses_before_any_runtime_or_process_mutation() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let configured_agents = home.join("agents");
    std::fs::create_dir_all(&configured_agents).unwrap();
    let attachment_link = fixture.path().join("attachments-link");
    std::os::unix::fs::symlink(&configured_agents, &attachment_link).unwrap();
    let kept = seed_configuration(fixture.path(), &home, &attachment_link);
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));
    let before_state = std::fs::read(home.join("state.db")).unwrap();
    let before_nmp = std::fs::read(home.join("nmp.redb")).unwrap();

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        !reset.status.success(),
        "profile/config overlap must refuse reset"
    );
    assert_eq!(std::fs::read(home.join("state.db")).unwrap(), before_state);
    assert_eq!(std::fs::read(home.join("nmp.redb")).unwrap(), before_nmp);
    assert!(
        !home.join("daemon.inhibit").exists(),
        "preflight failure mutated process state"
    );
    for (path, bytes) in kept {
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "config changed: {}",
            path.display()
        );
    }
}

#[test]
fn nmp_store_symlink_into_configuration_refuses_before_mutation() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    seed_runtime(&home);
    let profile = home.join("agents/writer.json");
    std::os::unix::fs::symlink(&profile, home.join("nmp.redb")).unwrap();
    let before_state = std::fs::read(home.join("state.db")).unwrap();

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(!reset.status.success(), "NMP path escape must refuse reset");
    assert_eq!(std::fs::read(home.join("state.db")).unwrap(), before_state);
    assert!(!home.join("daemon.inhibit").exists());
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn fixed_runtime_symlink_outside_the_selected_home_refuses_before_mutation() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    let external_state = fixture.path().join("external-state.db");
    write(&external_state, b"external runtime bytes");
    std::os::unix::fs::symlink(&external_state, home.join("state.db")).unwrap();
    superseded_epoch_store(&home.join("nmp.redb"));
    let before_nmp = std::fs::read(home.join("nmp.redb")).unwrap();

    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);
    assert!(
        !reset.status.success(),
        "fixed path escape must refuse reset"
    );
    assert_eq!(
        std::fs::read(&external_state).unwrap(),
        b"external runtime bytes"
    );
    assert_eq!(std::fs::read(home.join("nmp.redb")).unwrap(), before_nmp);
    assert!(!home.join("daemon.inhibit").exists());
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn reset_refuses_while_the_selected_startup_lock_is_owned() {
    let fixture = tempfile::tempdir().expect("temporary root");
    let home = selected_home(fixture.path());
    let kept = seed_configuration(fixture.path(), &home, &home.join("tmp/attachments"));
    seed_runtime(&home);
    superseded_epoch_store(&home.join("nmp.redb"));

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(home.join("daemon.lock"))
        .unwrap();
    let _guard = nix::fcntl::Flock::lock(lock, nix::fcntl::FlockArg::LockExclusive).unwrap();
    let reset = command(fixture.path(), &["daemon", "reset-state", CONFIRM]);

    assert!(
        !reset.status.success(),
        "owned state must not be reset concurrently"
    );
    assert!(home.join("state.db").exists());
    assert!(home.join("nmp.redb").exists());
    assert!(home.join("sessions/one/hook-calls.jsonl").exists());
    assert!(home.join("tmp/attachments/received.bin").exists());
    assert!(
        home.join("daemon.inhibit").exists(),
        "preflight completed, so the deliberate respawn inhibitor must remain"
    );
    for (path, bytes) in kept {
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
}
