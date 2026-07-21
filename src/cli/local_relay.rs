//! Lifecycle for the local relay started by `mosaico setup`.
//!
//! The public `mosaico relay` command remains a foreground command. Setup owns
//! only children it starts itself, records their exact PID, and never signals a
//! process whose command line does not point into this Mosaico home's relay bin.

use anyhow::{bail, Context, Result};
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const HOST: &str = "127.0.0.1";
const PORT: u16 = 9888;
const START_TIMEOUT: Duration = Duration::from_secs(10);
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize, Deserialize)]
struct RelayProcess {
    pid: u32,
    endpoint: String,
}

pub(super) fn start(owner_pubkey: &str, dry_run: bool) -> Result<()> {
    let paths = Paths::new();
    if let Some(record) = read_record(&paths.record)? {
        if process_is_owned(record.pid, &paths.bin_dir)? {
            println!("local relay already running at {}", record.endpoint);
            return Ok(());
        }
        if !dry_run {
            std::fs::remove_file(&paths.record).with_context(|| {
                format!("removing stale relay record {}", paths.record.display())
            })?;
        }
    }

    if dry_run {
        println!(
            "would start bundled local relay at ws://{HOST}:{PORT} (log: {})",
            paths.log.display()
        );
        return Ok(());
    }

    crate::config::ensure_dir(&paths.root)?;
    let stdout = open_log(&paths.log)?;
    let stderr = stdout
        .try_clone()
        .context("cloning local relay log handle")?;
    let executable = std::env::current_exe().context("resolving the mosaico executable")?;
    let mut command = Command::new(executable);
    command
        .args([
            "relay",
            "--host",
            HOST,
            "--port",
            &PORT.to_string(),
            "--owner-pubkey",
            owner_pubkey,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    detach(&mut command);
    let mut child = command.spawn().context("starting bundled local relay")?;
    let endpoint = format!("ws://{HOST}:{PORT}");
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().context("checking local relay process")? {
            bail!(
                "local relay exited with {status}; inspect {}",
                paths.log.display()
            );
        }
        if relay_accepts_connections() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "local relay did not accept connections within {START_TIMEOUT:?}; inspect {}",
                paths.log.display()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let write_result = write_record(
        &paths.record,
        &RelayProcess {
            pid: child.id(),
            endpoint: endpoint.clone(),
        },
    );
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error).context("recording the setup-owned local relay");
    }
    println!("started local relay at {endpoint} (pid {})", child.id());
    Ok(())
}

pub(super) fn stop(dry_run: bool) -> Result<()> {
    let paths = Paths::new();
    let Some(record) = read_record(&paths.record)? else {
        return Ok(());
    };
    if !process_exists(record.pid) {
        if !dry_run {
            std::fs::remove_file(&paths.record).with_context(|| {
                format!("removing stale relay record {}", paths.record.display())
            })?;
        }
        return Ok(());
    }
    if !process_is_owned(record.pid, &paths.bin_dir)? {
        bail!(
            "refusing to signal PID {}: it is not the relay owned by {}",
            record.pid,
            paths.root.display()
        );
    }
    if dry_run {
        println!("would stop owned local relay PID {}", record.pid);
        return Ok(());
    }

    let pid = pid(record.pid)?;
    kill(pid, Signal::SIGTERM).context("stopping owned local relay")?;
    let deadline = Instant::now() + STOP_TIMEOUT;
    while process_exists(record.pid) {
        if Instant::now() >= deadline {
            bail!("local relay PID {} did not stop after SIGTERM", record.pid);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    std::fs::remove_file(&paths.record)
        .with_context(|| format!("removing relay record {}", paths.record.display()))?;
    println!("stopped owned local relay PID {}", record.pid);
    Ok(())
}

pub(super) fn print_status() -> Result<()> {
    let paths = Paths::new();
    match read_record(&paths.record)? {
        Some(record) if process_is_owned(record.pid, &paths.bin_dir)? => {
            println!(
                "local relay    running  {}  pid={}",
                record.endpoint, record.pid
            );
        }
        Some(record) => println!("local relay    stale record  pid={}", record.pid),
        None => println!("local relay    not managed"),
    }
    Ok(())
}

struct Paths {
    root: PathBuf,
    bin_dir: PathBuf,
    record: PathBuf,
    log: PathBuf,
}

impl Paths {
    fn new() -> Self {
        let root = crate::config::mosaico_home().join("relay");
        Self {
            bin_dir: root.join("bin"),
            record: root.join("process.json"),
            log: root.join("relay.log"),
            root,
        }
    }
}

fn read_record(path: &Path) -> Result<Option<RelayProcess>> {
    let body = match std::fs::read_to_string(path) {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    serde_json::from_str(&body)
        .map(Some)
        .with_context(|| format!("parsing {}", path.display()))
}

fn write_record(path: &Path, record: &RelayProcess) -> Result<()> {
    let body = serde_json::to_vec_pretty(record).context("serializing local relay record")?;
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

fn open_log(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))
}

fn relay_accepts_connections() -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], PORT));
    TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok()
}

fn process_exists(raw_pid: u32) -> bool {
    pid(raw_pid).ok().is_some_and(|pid| kill(pid, None).is_ok())
}

fn process_is_owned(raw_pid: u32, bin_dir: &Path) -> Result<bool> {
    if !process_exists(raw_pid) {
        return Ok(false);
    }
    let output = Command::new("ps")
        .args(["-p", &raw_pid.to_string(), "-o", "command="])
        .output()
        .context("inspecting recorded local relay PID")?;
    if !output.status.success() {
        return Ok(false);
    }
    let command = String::from_utf8_lossy(&output.stdout);
    Ok(command.contains(&bin_dir.to_string_lossy().to_string()))
}

fn pid(raw_pid: u32) -> Result<Pid> {
    let value = i32::try_from(raw_pid).context("local relay PID exceeds platform range")?;
    if value <= 1 {
        bail!("refusing unsafe local relay PID {value}");
    }
    Ok(Pid::from_raw(value))
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

#[cfg(not(unix))]
fn detach(_command: &mut Command) {}

#[cfg(test)]
#[path = "local_relay/tests.rs"]
mod tests;
