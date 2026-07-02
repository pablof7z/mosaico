//! Thin client: connect to the per-machine daemon, spawning it if absent.
//!
//! Mechanics (docs/daemon-design.md §4):
//!   - try to connect to the UDS; if it answers, handshake and use it.
//!   - else acquire the startup `flock`, re-check (a racer may have just bound),
//!     reclaim a stale socket if present, spawn a detached daemon, release the
//!     lock, and poll-connect.
//!   - handshake carries a protocol version; a newer client that finds an older
//!     daemon asks it to exit, then respawns the new binary's daemon.

use super::protocol::{
    client_hello, daemon_too_new_message, handshake_decision, please_exit, HandshakeDecision,
    Request, Response, Welcome, DAEMON_HANDSHAKE_IO_TIMEOUT, DAEMON_RESPAWN_GRACE,
    DAEMON_STARTUP_TIMEOUT,
};
use super::{lock_path, socket_path};
use crate::config;
use anyhow::{bail, Context, Result};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{
    unix::{OwnedReadHalf, OwnedWriteHalf},
    UnixStream,
};

mod startup;

use startup::spawn_daemon_if_absent;
pub use startup::StartupLock;

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
                    tokio::time::sleep(DAEMON_RESPAWN_GRACE).await;
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
    /// Silently drops any `item` progress frames emitted before the terminal frame.
    pub async fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.send(method, params).await?;
        loop {
            let resp = self
                .read_frame()
                .await?
                .context("daemon closed the connection")?;
            if resp.id != id {
                bail!("response id mismatch: got {}, want {id}", resp.id);
            }
            if resp.item.is_some() {
                continue; // progress frame — wait for the terminal ok/error
            }
            if let Some(err) = resp.error {
                bail!("{}", err.message);
            }
            return resp.ok.context("daemon returned neither ok nor error");
        }
    }

    /// One-shot request that may emit progress `item` frames before the terminal
    /// `ok`/`error`. Used by slow startup hooks so the caller can show where the
    /// daemon is spending time without corrupting stdout protocols.
    pub async fn call_with_items<F>(
        &mut self,
        method: &str,
        params: serde_json::Value,
        mut on_item: F,
    ) -> Result<serde_json::Value>
    where
        F: FnMut(serde_json::Value),
    {
        let id = self.send(method, params).await?;
        loop {
            let resp = self
                .read_frame()
                .await?
                .context("daemon closed the connection")?;
            if resp.id != id {
                continue;
            }
            if let Some(item) = resp.item {
                on_item(item);
                continue;
            }
            if let Some(err) = resp.error {
                bail!("{}", err.message);
            }
            return resp.ok.context("daemon returned neither ok nor error");
        }
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
                bail!("{}", err.message);
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
        let stream = tokio::time::timeout(
            DAEMON_HANDSHAKE_IO_TIMEOUT,
            UnixStream::connect(socket_path()),
        )
        .await
        .context("timed out connecting to daemon socket")?
        .context("connecting to daemon socket")?;
        let (rh, wh) = stream.into_split();
        let mut reader = BufReader::new(rh);
        let mut writer = wh;

        // Send hello, read welcome.
        write_line(&mut writer, &client_hello()).await?;
        let welcome: Welcome =
            tokio::time::timeout(DAEMON_HANDSHAKE_IO_TIMEOUT, read_line(&mut reader))
                .await
                .context("timed out waiting for daemon welcome")??
                .context("daemon closed before welcome")?;

        match handshake_decision(welcome.protocol) {
            HandshakeDecision::Ready => Ok(ConnectOutcome::Ready(Client {
                reader,
                writer,
                next_id: 0,
            })),
            HandshakeDecision::AskOlderDaemonToExit => {
                // Older daemon under a newer binary (the human cutover): ask it
                // to exit so we can respawn the new binary's daemon.
                write_line(&mut writer, &please_exit()).await?;
                let _ = writer.flush().await;
                Ok(ConnectOutcome::SkewExitRequested)
            }
            HandshakeDecision::DaemonTooNew {
                daemon_protocol,
                client_protocol,
            } => bail!(
                "{}",
                daemon_too_new_message(daemon_protocol, client_protocol)
            ),
        }
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
