//! Exact local relay child and its isolated data directory.

use std::fs::File;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

pub struct RelayFixture {
    child: Child,
    url: String,
    data_dir: PathBuf,
}

impl std::fmt::Debug for RelayFixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelayFixture")
            .field("url", &self.url)
            .field("data_dir", &self.data_dir)
            .finish()
    }
}

impl RelayFixture {
    pub fn start_nak(sandbox: &Path) -> Result<Self> {
        const MAX_BIND_ATTEMPTS: usize = 4;

        for attempt in 1..=MAX_BIND_ATTEMPTS {
            let port = free_port()?;
            let data_dir = sandbox.join(format!("nak-{port}-{attempt}"));
            std::fs::create_dir_all(&data_dir)?;
            let stdout = File::create(data_dir.join("stdout.log"))?;
            let stderr = File::create(data_dir.join("stderr.log"))?;
            let child = Command::new(nak_bin())
                .arg("serve")
                .arg("--port")
                .arg(port.to_string())
                .arg("--quiet")
                .stdout(Stdio::from(stdout))
                .stderr(Stdio::from(stderr))
                .spawn()
                .context("spawn external nak relay")?;
            match wait_ready(child, port, data_dir) {
                Ok(relay) => return Ok(relay),
                Err(error)
                    if attempt < MAX_BIND_ATTEMPTS
                        && error
                            .to_string()
                            .to_ascii_lowercase()
                            .contains("address already in use") =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        unreachable!("the final nak startup attempt returns a relay or an error")
    }

    pub fn start_croissant(sandbox: &Path) -> Result<Self> {
        let binary = std::env::var_os("NIP29_RELAY_BIN")
            .map(PathBuf::from)
            .context("@croissant scenario requires NIP29_RELAY_BIN")?;
        let port = free_port()?;
        let data_dir = sandbox.join(format!("croissant-{port}"));
        std::fs::create_dir_all(&data_dir)?;
        let stdout = File::create(data_dir.join("stdout.log"))?;
        let stderr = File::create(data_dir.join("stderr.log"))?;
        let child = Command::new(binary)
            .env("PORT", port.to_string())
            .env("HOST", "127.0.0.1")
            .env("DOMAIN", "")
            .env("DATAPATH", &data_dir)
            .env(
                "OWNER_PUBLIC_KEY",
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .context("spawn external Croissant relay")?;
        wait_ready(child, port, data_dir)
    }

    pub fn url(&self) -> &str {
        &self.url
    }
}

impl Drop for RelayFixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_ready(mut child: Child, port: u16, data_dir: PathBuf) -> Result<RelayFixture> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(RelayFixture {
                child,
                url: format!("ws://127.0.0.1:{port}"),
                data_dir,
            });
        }
        if let Some(status) = child.try_wait()? {
            anyhow::bail!(
                "relay exited before readiness with {status}\n{}",
                relay_logs(&data_dir)
            );
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let status = child
                .wait()
                .map(|status| status.to_string())
                .unwrap_or_else(|error| format!("wait failed: {error}"));
            anyhow::bail!(
                "relay did not listen on port {port} before deadline; status={status}\n{}",
                relay_logs(&data_dir)
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn relay_logs(data_dir: &Path) -> String {
    ["stdout.log", "stderr.log"]
        .into_iter()
        .map(|name| {
            let body = std::fs::read_to_string(data_dir.join(name)).unwrap_or_default();
            format!("{name}:\n{body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

pub(super) fn nak_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("NAK") {
        return PathBuf::from(path);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = PathBuf::from(home).join("go/bin/nak");
        if candidate.is_file() {
            return candidate;
        }
    }
    PathBuf::from("nak")
}
