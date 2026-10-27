//! `nak serve`: the plain in-memory relay most tests share.

use super::{free_port, graceful_stop, relay_failure_message, TestRelay};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

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

impl TestRelay {
    pub fn start() -> Self {
        const MAX_BIND_ATTEMPTS: usize = 4;

        for attempt in 1..=MAX_BIND_ATTEMPTS {
            match Self::start_nak_attempt(attempt) {
                Ok(relay) => return relay,
                Err(message)
                    if attempt < MAX_BIND_ATTEMPTS
                        && message
                            .to_ascii_lowercase()
                            .contains("address already in use") =>
                {
                    continue;
                }
                Err(message) => panic!("{message}"),
            }
        }
        unreachable!("the final nak startup attempt returns or panics")
    }

    fn start_nak_attempt(attempt: usize) -> Result<Self, String> {
        let port = free_port();
        let bin = nak_bin();
        let scratch = std::env::temp_dir().join(format!(
            "nak-relay-test-{}-{port}-{attempt}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("create nak relay diagnostic directory");
        let stdout_path = scratch.join("relay.stdout.log");
        let stderr_path = scratch.join("relay.stderr.log");
        let stdout = std::fs::File::create(&stdout_path).expect("create nak relay stdout log");
        let stderr = std::fs::File::create(&stderr_path).expect("create nak relay stderr log");
        // `nak serve` loads events from stdin before it listens whenever stdin
        // is not a terminal, so an inherited open pipe (a CI runner, a piped
        // shell) keeps it from ever binding. Give it /dev/null instead.
        //
        // Shared relays live in process-lifetime statics whose destructors
        // never run. A watcher stops the relay when its lifetime pipe closes,
        // which also happens when the test process dies, so no relay
        // outlives the run that started it.
        let mut child = graceful_stop::watched_command(&bin)
            .arg("serve")
            .arg("--port")
            .arg(port.to_string())
            .arg("--quiet")
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .expect("spawn `nak serve` (is nak installed?)");
        let lifetime = child.stdin.take();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                // The shared relay is held in a process-lifetime `OnceLock`, so
                // its destructor does not run. Startup logs have served their
                // purpose once readiness succeeds; remove their directory now.
                let _ = std::fs::remove_dir_all(&scratch);
                return Ok(TestRelay {
                    child,
                    lifetime,
                    url: format!("ws://127.0.0.1:{port}"),
                    cleanup_dir: None,
                    croissant: false,
                });
            }
            if let Some(status) = child.try_wait().expect("poll nak relay") {
                let message = relay_failure_message(
                    "nak serve",
                    &bin,
                    port,
                    &scratch,
                    &status.to_string(),
                    &stdout_path,
                    &stderr_path,
                );
                let _ = std::fs::remove_dir_all(&scratch);
                return Err(message);
            }
            if Instant::now() > deadline {
                drop(lifetime);
                let status = child
                    .wait()
                    .map(|status| status.to_string())
                    .unwrap_or_else(|error| format!("wait failed: {error}"));
                let message = relay_failure_message(
                    "nak serve",
                    &bin,
                    port,
                    &scratch,
                    &status,
                    &stdout_path,
                    &stderr_path,
                );
                let _ = std::fs::remove_dir_all(&scratch);
                return Err(message);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
