use super::log_path;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

/// Fork a detached `mosaico daemon`: own session (`setsid` via
/// `process_group(0)`), stdio → daemon.log, survives the parent exiting.
///
/// The binary is `current_exe()` so an upgraded binary spawns its own daemon
/// (the basis of the version-skew re-exec). `$MOSAICO_BIN` overrides it —
/// used by tests (whose `current_exe()` is the test harness) and as an escape
/// hatch. Cargo test harnesses are rejected even when selected explicitly: a
/// harness interprets `daemon` as a test-name filter and can recursively run a
/// daemon-spawning test until the machine's process limit is exhausted.
///
/// Returns the `Child` handle (not detached from *this* process's `wait()`
/// perspective — only `process_group(0)` detaches it from the controlling
/// terminal/session) so the caller can `try_wait()` for a fast crash instead
/// of blindly polling the socket for the full startup timeout.
pub(super) fn spawn_detached_daemon() -> Result<std::process::Child> {
    let current_exe = std::env::current_exe().context("locating own executable")?;
    let exe = daemon_executable(
        std::env::var_os("MOSAICO_BIN").map(PathBuf::from),
        current_exe,
    )?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .context("opening daemon.log")?;
    let log_err = log.try_clone()?;
    let mut command = std::process::Command::new(&exe);
    command
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command
        .spawn()
        .with_context(|| format!("spawning detached daemon from {}", exe.display()))
}

fn daemon_executable(override_exe: Option<PathBuf>, current_exe: PathBuf) -> Result<PathBuf> {
    let exe = override_exe.unwrap_or(current_exe);
    if is_cargo_test_harness(&exe) {
        bail!(
            "refusing to spawn Cargo test harness {} as the daemon; set MOSAICO_BIN to the standalone mosaico binary",
            exe.display()
        );
    }
    Ok(exe)
}

fn is_cargo_test_harness(exe: &std::path::Path) -> bool {
    if exe.parent().and_then(|path| path.file_name()) != Some(std::ffi::OsStr::new("deps")) {
        return false;
    }
    let Some(name) = exe.file_stem().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some((_, hash)) = name.rsplit_once('-') else {
        return false;
    };
    hash.len() >= 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn standalone_binary_is_accepted() {
        let exe = PathBuf::from("/workspace/target/debug/mosaico");
        assert_eq!(daemon_executable(None, exe.clone()).unwrap(), exe);
    }

    #[test]
    fn cargo_test_harness_is_rejected_as_current_executable() {
        let harness = PathBuf::from("/workspace/target/debug/deps/mosaico-627aa7ddd0628899");
        let error = daemon_executable(None, harness).unwrap_err().to_string();
        assert!(error.contains("refusing to spawn Cargo test harness"));
    }

    #[test]
    fn cargo_test_harness_is_rejected_as_override() {
        let harness =
            PathBuf::from("/workspace/target/debug/deps/daemon_integration-a4aa00b9a4e9c2cc");
        let error = daemon_executable(
            Some(harness),
            Path::new("/workspace/target/debug/mosaico").to_path_buf(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("set MOSAICO_BIN to the standalone mosaico binary"));
    }
}
