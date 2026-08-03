use super::Home;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

/// Full teardown for an isolated test home: reap PTY supervisors first (they
/// intentionally outlive ordinary daemon restarts), then stop the daemon, then
/// force-kill any leftover daemon still bound to this exact `MOSAICO_HOME`.
pub(crate) fn stop_daemon(home: &Home) {
    // Ensure the public PTY reaper reads *this* home's metadata, not a sibling
    // test's, even if a previous Drop restored the process env early.
    // SAFETY: daemon_integration tests serialize env mutation via ENV_LOCK.
    unsafe {
        std::env::set_var("MOSAICO_HOME", home.dir.path());
    }
    let reap = mosaico::pty::reap_home_supervisors().unwrap_or_else(|error| {
        panic!("PTY reap failed during test teardown: {error:#}");
    });
    if !reap.is_clean() {
        panic!(
            "PTY supervisors survived test teardown: {}",
            reap.errors.join("; ")
        );
    }
    request_version_skew_exit(home);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && home.sock().exists() {
        std::thread::sleep(Duration::from_millis(25));
    }
    force_kill_daemons_for_home(home.dir.path());
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && home.sock().exists() {
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !home.sock().exists(),
        "daemon did not complete orderly shutdown before the deadline"
    );
    scavenge_deleted_tmp_mosaico_processes();
}

fn request_version_skew_exit(home: &Home) {
    if let Ok(stream) = UnixStream::connect(home.sock()) {
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);
        let _ = writeln!(
            writer,
            "{}",
            serde_json::json!({"protocol": u32::MAX, "client_version": "t"})
        );
        let mut welcome = String::new();
        let _ = reader.read_line(&mut welcome);
        let _ = writeln!(writer, "{}", serde_json::json!({"protocol": u32::MAX}));
        let mut response = String::new();
        let _ = reader.read_line(&mut response);
    }
}

/// Kill every `mosaico daemon` whose `/proc/PID/environ` lists this exact home.
/// Catches the case where the UDS is already gone but a detached daemon still
/// lives (or the version-skew path never connected).
fn force_kill_daemons_for_home(home: &Path) {
    let home = home.to_string_lossy();
    let needle = format!("MOSAICO_HOME={home}");
    for pid in mosaico_daemon_pids() {
        if process_environ(pid).is_some_and(|env| env.contains(&needle)) {
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
}

/// Reap mosaico daemon/supervisor processes whose executable was deleted from
/// under `/tmp` — the classic leak after a TempDir drop without Drop-path
/// teardown (SIGKILL'd cargo test, OOM, etc.). Never touches
/// `~/.local/bin/mosaico` or other non-tmp installs.
pub(crate) fn scavenge_deleted_tmp_mosaico_processes() {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for entry in entries.flatten() {
        let pid: i32 = match entry.file_name().to_string_lossy().parse() {
            Ok(pid) if pid > 1 => pid,
            _ => continue,
        };
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            continue;
        };
        let exe = exe.to_string_lossy();
        // Linux reports deleted binaries as "/path/foo (deleted)".
        if !(exe.contains("(deleted)") && exe.contains("/tmp/") && exe.contains("mosaico")) {
            continue;
        }
        let Some(cmdline) = process_cmdline(pid) else {
            continue;
        };
        let args: Vec<&str> = cmdline
            .split('\0')
            .filter(|part| !part.is_empty())
            .collect();
        let role = args.get(1).copied().unwrap_or("");
        if role != "daemon" && role != "__pty-supervisor" {
            continue;
        }
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
    }
}

fn mosaico_daemon_pids() -> Vec<i32> {
    let Ok(output) = std::process::Command::new("/bin/ps")
        .args(["-eo", "pid=", "-o", "args="])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let (pid, args) = line.split_once(char::is_whitespace)?;
            let pid: i32 = pid.parse().ok()?;
            let tokens: Vec<&str> = args.split_whitespace().collect();
            let is_daemon = tokens.iter().any(|arg| arg.ends_with("mosaico"))
                && tokens.contains(&"daemon")
                && !tokens.contains(&"__pty-supervisor");
            is_daemon.then_some(pid)
        })
        .collect()
}

fn process_environ(pid: i32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
    Some(String::from_utf8_lossy(&bytes).replace('\0', "\n"))
}

fn process_cmdline(pid: i32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
