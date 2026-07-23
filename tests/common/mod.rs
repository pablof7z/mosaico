//! Shared test harness: spin up a real in-memory relay via `nak serve`.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct TestRelay {
    child: Child,
    pub url: String,
    /// Data dir to remove on drop (NIP-29 relay only). `nak serve` is in-memory
    /// and leaves nothing behind, so it stays `None`. Without this, every
    /// `start_nip29_relay` leaked its `nip29-relay-test-<port>` dir — thousands
    /// accumulated across runs and, combined with the relay's 100 GB LMDB map
    /// reservation, eventually starved the temp filesystem.
    data_dir: Option<PathBuf>,
}

pub(crate) fn nak_bin() -> PathBuf {
    if let Ok(p) = std::env::var("NAK") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let candidate = PathBuf::from(&home).join("go/bin/nak");
    if candidate.exists() {
        return candidate;
    }
    PathBuf::from("nak") // rely on PATH
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn tail_file(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    if bytes.is_empty() {
        return "<empty>".to_string();
    }

    let text = String::from_utf8_lossy(&bytes);
    let mut lines = text.lines().rev().take(40).collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n")
}

fn nip29_failure_message(
    bin: &Path,
    port: u16,
    data: &Path,
    status: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    format!(
        "NIP-29 relay did not come up on port {port}\n\
         binary: {}\n\
         data: {}\n\
         status: {status}\n\
         stdout ({}):\n{}\n\
         stderr ({}):\n{}",
        bin.display(),
        data.display(),
        stdout_path.display(),
        tail_file(stdout_path),
        stderr_path.display(),
        tail_file(stderr_path)
    )
}

/// Path to the NIP-29 relay binary — `nak serve` does NOT implement NIP-29
/// group semantics (9007/9002 creates, 39001 admin reflection), so any test
/// that owns groups or mints subgroups must run against a real NIP-29 relay.
/// Supply the externally installed executable with `$NIP29_RELAY_BIN`. Linux
/// can also resolve `croissant` from PATH. macOS requires the explicit variable
/// because relay builds without `MDB_NOLOCK` leak POSIX named semaphores when
/// the test harness terminates them.
#[allow(dead_code)]
fn nip29_relay_bin() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("NIP29_RELAY_BIN") {
        return Ok(PathBuf::from(p));
    }
    #[cfg(target_os = "macos")]
    {
        Err(
            "set $NIP29_RELAY_BIN to an external MDB_NOLOCK Croissant build; \
             implicit relay discovery is disabled on macOS because ordinary \
             LMDB builds leak POSIX named semaphores under test shutdown. \
             See mosaico #329."
                .to_string(),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        find_on_path("croissant").ok_or_else(|| {
            "Croissant is external infrastructure; set $NIP29_RELAY_BIN or install \
             `croissant` on PATH."
                .to_string()
        })
    }
}

#[cfg(not(target_os = "macos"))]
fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

impl TestRelay {
    /// Spawn a real NIP-29 relay on an ephemeral port with an isolated data dir.
    /// Use for daemon tests that exercise group ownership / subgroup minting.
    #[allow(dead_code)]
    pub fn start_nip29_relay() -> Self {
        let port = free_port();
        let bin = nip29_relay_bin().unwrap_or_else(|msg| panic!("{msg}"));
        assert!(
            bin.exists(),
            "NIP-29 relay binary not found at {} (set $NIP29_RELAY_BIN)",
            bin.display()
        );
        let data = std::env::temp_dir().join(format!("nip29-relay-test-{port}"));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).expect("create NIP-29 relay data dir");
        let stdout_path = data.join("relay.stdout.log");
        let stderr_path = data.join("relay.stderr.log");
        let stdout = std::fs::File::create(&stdout_path).expect("create NIP-29 relay stdout log");
        let stderr = std::fs::File::create(&stderr_path).expect("create NIP-29 relay stderr log");
        let mut child = Command::new(&bin)
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("DATAPATH", &data)
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn NIP-29 relay");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            // On the startup-failure paths the `TestRelay` is never constructed,
            // so `Drop` can't reclaim the data dir. Build the message (it reads
            // the log files under `data`) BEFORE removing the dir, then panic.
            if let Some(status) = child.try_wait().expect("poll NIP-29 relay") {
                let msg = nip29_failure_message(
                    &bin,
                    port,
                    &data,
                    &status.to_string(),
                    &stdout_path,
                    &stderr_path,
                );
                let _ = std::fs::remove_dir_all(&data);
                panic!("{msg}");
            }
            if Instant::now() > deadline {
                let msg = nip29_failure_message(
                    &bin,
                    port,
                    &data,
                    "still running after startup deadline",
                    &stdout_path,
                    &stderr_path,
                );
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_dir_all(&data);
                panic!("{msg}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        TestRelay {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            data_dir: Some(data),
        }
    }
}

impl TestRelay {
    pub fn start() -> Self {
        let port = free_port();
        let child = Command::new(nak_bin())
            .arg("serve")
            .arg("--port")
            .arg(port.to_string())
            .arg("--quiet")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn `nak serve` (is nak installed?)");

        // Wait for the relay to accept TCP connections.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            if Instant::now() > deadline {
                panic!("nak serve did not come up on port {port}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        TestRelay {
            child,
            // Match the address used by the readiness probe. Keeping the URL
            // numeric avoids an IPv6 localhost resolution when the relay is
            // listening only on 127.0.0.1.
            url: format!("ws://127.0.0.1:{port}"),
            data_dir: None,
        }
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        // Reclaim the relay's data dir so repeated runs don't leak thousands of
        // `nip29-relay-test-<port>` trees (each with a large sparse LMDB map).
        if let Some(dir) = &self.data_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
