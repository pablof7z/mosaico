use super::{bin, Home};

/// Spawn the daemon by exec'ing the real binary (not current_exe of the test).
pub(super) fn spawn_real_daemon(home: &Home) -> std::process::Child {
    spawn_real_daemon_with_env(home, &[])
}

pub(super) fn spawn_real_daemon_with_env(home: &Home, env: &[(&str, &str)]) -> std::process::Child {
    let log = std::fs::File::create(home.dir.path().join("daemon.log")).unwrap();
    let mut command = std::process::Command::new(bin());
    command
        .arg("daemon")
        .env_remove("MOSAICO")
        .env("MOSAICO_HOME", home.dir.path())
        .env("MOSAICO_CONFIG", home.dir.path().join("config.json"))
        .env("MOSAICO_DAEMON_GRACE_S", "30")
        .env(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1")
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    for (key, value) in env {
        command.env(key, value);
    }
    command.spawn().expect("spawn daemon")
}
