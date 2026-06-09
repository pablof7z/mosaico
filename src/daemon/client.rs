//! Thin client: connect to the per-machine daemon, spawning it if absent.
//!
//! Mechanics (docs/daemon-design.md §4):
//!   - try to connect to the UDS; if it answers, handshake and use it.
//!   - else acquire the startup `flock`, re-check (a racer may have just bound),
//!     reclaim a stale socket if present, spawn a detached daemon, release the
//!     lock, and poll-connect.
//!   - handshake carries a protocol version; a newer client that finds an older
//!     daemon asks it to exit, then respawns the new binary's daemon.

use super::protocol::{protocol_version, Hello, PleaseExit, Request, Response, Welcome};
use super::{lock_path, log_path, socket_path};
use crate::config;
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{
    unix::{OwnedReadHalf, OwnedWriteHalf},
    UnixStream,
};

const SPAWN_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A live connection to the daemon, post-handshake.
pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    /// Connect to the running daemon, spawning one (and re-execing on version
    /// skew) as needed. This is the single entry point every thin verb uses.
    ///
    /// Each iteration: try to connect+handshake. A `Ready` returns. A skew-exit
    /// or a connect failure both lead to `spawn_daemon_if_absent` (which is a
    /// no-op if a daemon is already up), then retry — so a post-skew respawn of
    /// the *new* binary's daemon always happens.
    pub async fn connect_or_spawn() -> Result<Client> {
        let mut last_err: Option<anyhow::Error> = None;
        for _ in 0..5 {
            match Self::try_connect_handshake().await {
                Ok(ConnectOutcome::Ready(c)) => return Ok(c),
                Ok(ConnectOutcome::SkewExitRequested) => {
                    // The old daemon is exiting; let it release the socket, then
                    // (re)spawn the new binary's daemon.
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    spawn_daemon_if_absent().await?;
                }
                Err(e) => {
                    // A protocol-too-new error (newer daemon, older client) is a
                    // hard stop — don't keep retrying.
                    if e.to_string().contains("is newer than this binary") {
                        return Err(e);
                    }
                    last_err = Some(e);
                    spawn_daemon_if_absent().await?;
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("could not establish a daemon connection")))
    }

    /// One-shot request → single `ok` result (errors map to `Err`).
    pub async fn call(&mut self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let id = self.send(method, params).await?;
        let resp = self.read_frame().await?.context("daemon closed the connection")?;
        if resp.id != id {
            bail!("response id mismatch: got {}, want {id}", resp.id);
        }
        if let Some(err) = resp.error {
            bail!("daemon error [{}]: {}", err.code, err.message);
        }
        resp.ok.context("daemon returned neither ok nor error")
    }

    /// Streaming request: returns each `item` to `on_item` until the daemon ends
    /// the stream or the connection drops. Used by `tail`.
    pub async fn stream<F: FnMut(serde_json::Value)>(
        &mut self,
        method: &str,
        params: serde_json::Value,
        mut on_item: F,
    ) -> Result<()> {
        let id = self.send(method, params).await?;
        loop {
            let Some(frame) = self.read_frame().await? else {
                return Ok(()); // daemon closed
            };
            if frame.id != id {
                continue;
            }
            if let Some(err) = frame.error {
                bail!("daemon error [{}]: {}", err.code, err.message);
            }
            if frame.end.unwrap_or(false) {
                return Ok(());
            }
            if let Some(item) = frame.item {
                on_item(item);
            }
        }
    }

    async fn send(&mut self, method: &str, params: serde_json::Value) -> Result<u64> {
        self.next_id += 1;
        let id = self.next_id;
        let req = Request {
            id,
            method: method.to_string(),
            params,
        };
        write_line(&mut self.writer, &req).await?;
        Ok(id)
    }

    async fn read_frame(&mut self) -> Result<Option<Response>> {
        read_line(&mut self.reader).await
    }

    // ── handshake / connect ──────────────────────────────────────────────

    async fn try_connect_handshake() -> Result<ConnectOutcome> {
        let stream = UnixStream::connect(socket_path())
            .await
            .context("connecting to daemon socket")?;
        let (rh, wh) = stream.into_split();
        let mut reader = BufReader::new(rh);
        let mut writer = wh;

        // Send hello, read welcome.
        write_line(
            &mut writer,
            &Hello {
                protocol: protocol_version(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            },
        )
        .await?;
        let welcome: Welcome = read_line(&mut reader)
            .await?
            .context("daemon closed before welcome")?;

        if welcome.protocol == protocol_version() {
            return Ok(ConnectOutcome::Ready(Client {
                reader,
                writer,
                next_id: 0,
            }));
        }
        if welcome.protocol < protocol_version() {
            // Older daemon under a newer binary (the human cutover): ask it to
            // exit so we can respawn the new binary's daemon.
            write_line(
                &mut writer,
                &PleaseExit {
                    protocol: protocol_version(),
                },
            )
            .await?;
            let _ = writer.flush().await;
            return Ok(ConnectOutcome::SkewExitRequested);
        }
        // Newer daemon, older client: don't bridge. Tell the human to restart.
        bail!(
            "daemon protocol {} is newer than this binary's {} — restart your tenex-edge session \
             (or reinstall) so client and daemon match",
            welcome.protocol,
            protocol_version()
        );
    }
}

enum ConnectOutcome {
    Ready(Client),
    SkewExitRequested,
}

// ── framing helpers (newline-delimited JSON) ─────────────────────────────────

async fn write_line<T: serde::Serialize>(w: &mut OwnedWriteHalf, v: &T) -> Result<()> {
    let mut line = serde_json::to_string(v)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

async fn read_line<T: serde::de::DeserializeOwned>(
    r: &mut BufReader<OwnedReadHalf>,
) -> Result<Option<T>> {
    let mut buf = String::new();
    let n = r.read_line(&mut buf).await?;
    if n == 0 {
        return Ok(None); // EOF
    }
    let v = serde_json::from_str(buf.trim_end()).context("parsing daemon frame")?;
    Ok(Some(v))
}

// ── spawn-if-absent (race-safe via flock) ────────────────────────────────────

/// Ensure a daemon is listening. Under the startup lock: re-check the socket,
/// reclaim a stale one, then spawn a detached daemon and poll-connect.
async fn spawn_daemon_if_absent() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;
    // Acquire the exclusive startup lock (blocks until other spawners finish).
    let lock = StartupLock::acquire()?;

    // Someone may have bound the socket while we waited for the lock.
    if probe_connect().await {
        return Ok(());
    }
    // Stale socket: file present but nobody answering → reclaim under the lock.
    let sock = socket_path();
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }

    spawn_detached_daemon()?;
    // Lock is released when `lock` drops (after spawn returns); the daemon
    // re-acquires it on its own startup.
    drop(lock);

    // Poll-connect until the daemon binds.
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    while Instant::now() < deadline {
        if probe_connect().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("daemon did not come up within {SPAWN_CONNECT_TIMEOUT:?}")
}

/// Cheap liveness probe: can we open the socket at all?
async fn probe_connect() -> bool {
    UnixStream::connect(socket_path()).await.is_ok()
}

/// Fork a detached `tenex-edge __daemon`: own session (`setsid` via
/// `process_group(0)`), stdio → daemon.log, survives the parent exiting.
///
/// The binary is `current_exe()` so an upgraded binary spawns its own daemon
/// (the basis of the version-skew re-exec). `$TENEX_EDGE_BIN` overrides it —
/// used by tests (whose `current_exe()` is the test harness) and as an escape
/// hatch.
fn spawn_detached_daemon() -> Result<()> {
    let exe = match std::env::var_os("TENEX_EDGE_BIN") {
        Some(p) => PathBuf::from(p),
        None => std::env::current_exe().context("locating own executable")?,
    };
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path())
        .context("opening daemon.log")?;
    let log_err = log.try_clone()?;
    let mut command = std::process::Command::new(exe);
    command
        .arg("__daemon")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log))
        .stderr(std::process::Stdio::from(log_err));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0); // detach from the caller's process group
    }
    command.spawn().context("spawning detached daemon")?;
    Ok(())
}

/// RAII wrapper over an exclusive `flock` on `daemon.lock`. The lock is released
/// when the `Flock` guard drops (i.e. when this `StartupLock` drops).
pub struct StartupLock {
    _flock: nix::fcntl::Flock<std::fs::File>,
}

fn open_lock_file() -> Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(lock_path())
        .context("opening daemon.lock")
}

impl StartupLock {
    /// Blocking exclusive acquire (used by spawning clients).
    pub fn acquire() -> Result<Self> {
        let file = open_lock_file()?;
        let flock = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive)
            .map_err(|(_, e)| anyhow::anyhow!("flock daemon.lock: {e}"))?;
        Ok(StartupLock { _flock: flock })
    }

    /// Non-blocking exclusive acquire: `Ok(Some)` if we got it, `Ok(None)` if
    /// held by a live daemon. Used by the daemon to detect an existing peer.
    pub fn try_acquire() -> Result<Option<Self>> {
        let file = open_lock_file()?;
        match nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock) {
            Ok(flock) => Ok(Some(StartupLock { _flock: flock })),
            // EWOULDBLOCK (== EAGAIN on these platforms): another daemon holds it.
            Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(None),
            Err((_, e)) => Err(anyhow::anyhow!("flock(NB) daemon.lock: {e}")),
        }
    }
}
