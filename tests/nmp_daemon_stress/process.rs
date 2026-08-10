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
        .args(["-p", &pid.to_string(), "-o", "rss=,time="])
        .output()
        .expect("sample daemon process");
    assert!(
        output.status.success(),
        "daemon process disappeared during sample"
    );
    let line = String::from_utf8_lossy(&output.stdout);
    let mut fields = line.split_whitespace();
    let rss_kib = fields
        .next()
        .expect("ps rss")
        .parse()
        .expect("numeric ps rss");
    let cpu_seconds = parse_cpu_time(fields.next().expect("ps cpu time"));
    ProcessSample {
        cpu_seconds,
        rss_kib,
        threads: thread_count(pid),
        file_descriptors: file_descriptor_count(pid),
    }
}

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
    use super::parse_cpu_time;

    #[test]
    fn parses_macos_process_cpu_time() {
        assert_eq!(parse_cpu_time("0:01.25"), 1.25);
        assert_eq!(parse_cpu_time("1:02:03.50"), 3_723.5);
        assert_eq!(parse_cpu_time("2-01:00:00.00"), 176_400.0);
    }
}
