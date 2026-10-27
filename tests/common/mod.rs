//! Shared test harness: spin up a real in-memory relay via `nak serve`, or a
//! NIP-29 relay (Croissant).

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "graceful_stop.rs"]
mod graceful_stop;
#[path = "nak_relay.rs"]
mod nak_relay;
#[allow(unused_imports)]
pub(crate) use nak_relay::nak_bin;

pub struct TestRelay {
    child: Child,
    /// Lifetime pipe of a watched relay: closing it (explicitly, or because
    /// the test process exited for any reason) stops the relay.
    lifetime: Option<ChildStdin>,
    pub url: String,
    /// Relay data or diagnostic scratch directory removed on drop.
    cleanup_dir: Option<PathBuf>,
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

fn relay_failure_message(
    label: &str,
    bin: &Path,
    port: u16,
    data: &Path,
    status: &str,
    stdout_path: &Path,
    stderr_path: &Path,
) -> String {
    format!(
        "{label} did not come up on port {port}\n\
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
/// because on macOS LMDB keeps its locks in POSIX named semaphores that only a
/// clean store close unlinks: the build must shut down gracefully on SIGTERM
/// (pablof7z/croissant#12), or every stopped relay leaks semaphores until
/// Croissant fails with `mdb_env_open: no space left on device` (#856).
#[allow(dead_code)]
fn nip29_relay_bin() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("NIP29_RELAY_BIN") {
        return Ok(PathBuf::from(p));
    }
    #[cfg(target_os = "macos")]
    {
        Err(
            "set $NIP29_RELAY_BIN to a Croissant build that closes its store on \
             SIGTERM; implicit relay discovery is disabled on macOS because older \
             builds leak LMDB POSIX named semaphores on every test shutdown. \
             See mosaico #856."
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
            .stdin(Stdio::null())
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
                let msg = relay_failure_message(
                    "NIP-29 relay",
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
                let msg = relay_failure_message(
                    "NIP-29 relay",
                    &bin,
                    port,
                    &data,
                    "still running after startup deadline",
                    &stdout_path,
                    &stderr_path,
                );
                graceful_stop::stop_gracefully(&mut child);
                let _ = std::fs::remove_dir_all(&data);
                panic!("{msg}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        TestRelay {
            child,
            lifetime: None,
            url: format!("ws://127.0.0.1:{port}"),
            cleanup_dir: Some(data),
        }
    }
}

impl Drop for TestRelay {
    fn drop(&mut self) {
        // A watched relay stops when its lifetime pipe closes; killing the
        // watcher shell instead would orphan the relay. An unwatched relay
        // (Croissant) gets SIGTERM so it closes LMDB.
        if self.lifetime.take().is_none() {
            graceful_stop::stop_gracefully(&mut self.child);
        }
        let _ = self.child.wait();
        if let Some(dir) = &self.cleanup_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
