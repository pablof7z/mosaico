use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub(super) struct ProcessSample {
    pub(super) cpu_seconds: f64,
    pub(super) rss_kib: u64,
    pub(super) threads: usize,
    pub(super) file_descriptors: usize,
}

pub(super) struct DaemonProcess {
    child: Child,
    socket: PathBuf,
}

impl DaemonProcess {
    pub(super) fn spawn(binary: &Path, home: &Path, config: &Path) -> Self {
        let log = std::fs::File::create(home.join("daemon.log")).expect("daemon log");
        let child = Command::new(binary)
            .arg("daemon")
            .env_remove("MOSAICO")
            .env("MOSAICO_HOME", home)
            .env("MOSAICO_CONFIG", config)
            .env("MOSAICO_DAEMON_GRACE_S", "30")
            .env(mosaico::pty::REAP_SESSIONS_ON_STOP_ENV, "1")
            .stdout(Stdio::from(log.try_clone().expect("clone daemon log")))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn standalone daemon");
        Self {
            child,
            socket: home.join("daemon.sock"),
        }
    }

    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn wait_ready(&mut self, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if rpc(&self.socket, "ping").is_ok() {
                return;
            }
            if let Some(status) = self.child.try_wait().expect("poll standalone daemon") {
                panic!("standalone daemon exited before ready: {status}");
            }
            assert!(
                Instant::now() < deadline,
                "standalone daemon startup timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub(super) fn call(&self, method: &str) -> serde_json::Value {
        rpc(&self.socket, method).unwrap_or_else(|error| panic!("RPC {method}: {error}"))
    }

    pub(super) fn log_tail(&self) -> String {
        let path = self
            .socket
            .parent()
            .expect("daemon socket has a parent")
            .join("daemon.log");
        let log = std::fs::read_to_string(path).unwrap_or_default();
        log.lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(super) fn shutdown(&mut self, timeout: Duration) {
        let response = self.call("shutdown");
        assert_eq!(response["stopped"], true);
        let deadline = Instant::now() + timeout;
        loop {
            if self
                .child
                .try_wait()
                .expect("poll standalone daemon shutdown")
                .is_some()
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "standalone daemon shutdown timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = rpc(&self.socket, "shutdown");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill(); // exact child PID, never a process-name match.
        let _ = self.child.wait();
    }
}

pub(super) fn sample(pid: u32) -> ProcessSample {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "rss="])
        .output()
        .expect("sample daemon process");
    assert!(
        output.status.success(),
        "daemon process disappeared during sample"
    );
    ProcessSample {
        cpu_seconds: cpu_seconds(pid),
        rss_kib: String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .expect("numeric ps rss"),
        threads: thread_count(pid),
        file_descriptors: file_descriptor_count(pid),
    }
}

/// Linux `ps` reports CPU time in whole seconds, too coarse for the steady
/// window's bound; `/proc/<pid>/stat` counts clock ticks.
#[cfg(target_os = "linux")]
fn cpu_seconds(pid: u32) -> f64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/stat: {error}"));
    // SAFETY: sysconf has no preconditions and only reads a constant.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    assert!(ticks_per_second > 0, "sysconf(_SC_CLK_TCK) failed");
    proc_stat_cpu_ticks(&stat) as f64 / ticks_per_second as f64
}

#[cfg(not(target_os = "linux"))]
fn cpu_seconds(pid: u32) -> f64 {
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "time="])
        .output()
        .expect("sample daemon cpu time");
    assert!(
        output.status.success(),
        "daemon process disappeared during sample"
    );
    parse_cpu_time(String::from_utf8_lossy(&output.stdout).trim())
}

/// User plus system clock ticks from a `/proc/<pid>/stat` line. The command
/// name may contain spaces and parentheses, so fields are counted after the
/// last `)`: utime and stime are fields 14 and 15 of the line.
fn proc_stat_cpu_ticks(stat: &str) -> u64 {
    let after_comm = &stat[stat.rfind(')').expect("stat command name") + 1..];
    let fields = after_comm.split_whitespace().collect::<Vec<_>>();
    let tick = |index: usize| -> u64 { fields[index].parse().expect("numeric stat ticks") };
    tick(11) + tick(12)
}

#[cfg(target_os = "linux")]
fn thread_count(pid: u32) -> usize {
    proc_entry_count(pid, "task")
}

#[cfg(not(target_os = "linux"))]
fn thread_count(pid: u32) -> usize {
    let output = Command::new("/bin/ps")
        .args(["-M", "-p", &pid.to_string()])
        .output()
        .expect("sample daemon threads");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .skip(1)
        .count()
}

#[cfg(target_os = "linux")]
fn file_descriptor_count(pid: u32) -> usize {
    proc_entry_count(pid, "fd")
}

#[cfg(not(target_os = "linux"))]
fn file_descriptor_count(pid: u32) -> usize {
    let output = Command::new("/usr/sbin/lsof")
        .args(["-p", &pid.to_string(), "-Fn"])
        .output()
        .expect("sample daemon file descriptors");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| line.starts_with('f'))
        .count()
}

/// Entries under `/proc/<pid>/<dir>`: one per thread (`task`) or open
/// descriptor (`fd`).
#[cfg(target_os = "linux")]
fn proc_entry_count(pid: u32, dir: &str) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/{dir}"))
        .unwrap_or_else(|error| panic!("read /proc/{pid}/{dir}: {error}"))
        .count()
}

fn parse_cpu_time(value: &str) -> f64 {
    let (days, clock) = value.split_once('-').map_or((0.0, value), |(days, clock)| {
        (days.parse::<f64>().expect("numeric ps cpu days"), clock)
    });
    let fields = clock
        .split(':')
        .map(|field| field.parse::<f64>().expect("numeric ps cpu field"))
        .collect::<Vec<_>>();
    let seconds = match fields.as_slice() {
        [minutes, seconds] => minutes * 60.0 + seconds,
        [hours, minutes, seconds] => hours * 3600.0 + minutes * 60.0 + seconds,
        _ => panic!("unexpected ps cpu time {value:?}"),
    };
    days * 86_400.0 + seconds
}

fn rpc(socket: &Path, method: &str) -> Result<serde_json::Value, String> {
    let stream = UnixStream::connect(socket).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let mut writer = stream.try_clone().map_err(|error| error.to_string())?;
    let mut reader = BufReader::new(stream);
    let protocol = mosaico::daemon::protocol::protocol_version();
    writeln!(
        writer,
        "{}",
        serde_json::json!({"protocol": protocol, "client_version": "stress"})
    )
    .map_err(|error| error.to_string())?;
    let mut welcome = String::new();
    reader
        .read_line(&mut welcome)
        .map_err(|error| error.to_string())?;
    let parsed: serde_json::Value =
        serde_json::from_str(welcome.trim()).map_err(|error| error.to_string())?;
    if parsed["protocol"] != protocol {
        return Err(format!("protocol mismatch: {parsed}"));
    }
    writeln!(
        writer,
        "{}",
        serde_json::json!({"id": 1, "method": method, "params": {}})
    )
    .map_err(|error| error.to_string())?;
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    let parsed: serde_json::Value =
        serde_json::from_str(response.trim()).map_err(|error| error.to_string())?;
    if let Some(error) = parsed.get("error") {
        return Err(error.to_string());
    }
    parsed
        .get("ok")
        .cloned()
        .ok_or_else(|| format!("RPC returned no result: {parsed}"))
}

#[cfg(test)]
mod tests {
    use super::{parse_cpu_time, proc_stat_cpu_ticks};

    #[test]
    fn reads_user_and_system_ticks_from_proc_stat() {
        let stat = "4242 (mosaico (d) x) S 1 4242 4242 0 -1 4194560 100 0 0 0 \
                    37 5 0 0 20 0 9 0 12345 1000000 500";
        assert_eq!(proc_stat_cpu_ticks(stat), 42);
    }

    #[test]
    fn parses_macos_process_cpu_time() {
        assert_eq!(parse_cpu_time("0:01.25"), 1.25);
        assert_eq!(parse_cpu_time("1:02:03.50"), 3_723.5);
        assert_eq!(parse_cpu_time("2-01:00:00.00"), 176_400.0);
    }
}
