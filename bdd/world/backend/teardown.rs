use std::io::Write as _;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use super::Backend;

impl Backend {
    pub(in crate::world) fn live_resources(&self) -> Vec<String> {
        let mut resources = Vec::new();
        if self.socket().exists() {
            resources.push(format!("daemon socket {}", self.socket().display()));
        }
        resources.extend(
            self.pty_metadata()
                .into_iter()
                .filter(|metadata| UnixStream::connect(&metadata.socket).is_ok())
                .map(|metadata| format!("PTY {}", metadata.id)),
        );
        resources
    }

    pub(super) fn stop_pty_supervisors(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let live = self
                .pty_metadata()
                .into_iter()
                .filter(|metadata| UnixStream::connect(&metadata.socket).is_ok())
                .collect::<Vec<_>>();
            if live.is_empty() {
                return;
            }
            for metadata in &live {
                if let Ok(mut stream) = UnixStream::connect(&metadata.socket) {
                    let _ = stream.write_all(b"KILL\n");
                    let _ = stream.flush();
                }
            }
            if Instant::now() >= deadline {
                eprintln!(
                    "BDD teardown could not stop PTY supervisors: {}",
                    live.iter()
                        .map(|metadata| metadata.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn pty_metadata(&self) -> Vec<mosaico::pty::LaunchMetadata> {
        let directory = self.mosaico_home.join("pty");
        let Ok(entries) = std::fs::read_dir(directory) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|entry| std::fs::read(entry.path()).ok())
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect()
    }

    pub(super) fn stop_daemon(&self) {
        if !self.socket().exists() {
            return;
        }
        let _ = self.run(&["daemon", "stop"], None, Duration::from_secs(10));
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.socket().exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
