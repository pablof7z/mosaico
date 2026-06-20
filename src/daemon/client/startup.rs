use super::*;

const SPAWN_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const HANDSHAKE_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

// ── spawn-if-absent (race-safe via flock) ────────────────────────────────────

/// Ensure a daemon is listening. Under the startup lock: re-check the socket,
/// reclaim a stale one, then spawn a detached daemon and poll-connect.
pub(super) async fn spawn_daemon_if_absent() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;

    let mut noted_wait = false;
    let wait_deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    while Instant::now() < wait_deadline {
        if probe_handshake().await {
            return Ok(());
        }
        if let Some(lock) = StartupLock::try_acquire()? {
            eprintln!("[tenex-edge] starting daemon...");
            let sock = socket_path();
            if sock.exists() {
                let _ = std::fs::remove_file(&sock);
            }
            spawn_detached_daemon()?;
            // Lock is released when `lock` drops (after spawn returns); the daemon
            // re-acquires it on its own startup.
            drop(lock);
            break;
        }
        if !noted_wait {
            eprintln!("[tenex-edge] waiting for daemon to finish startup...");
            noted_wait = true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Poll until the daemon accepts and answers handshakes.
    let mut noted_ready = false;
    let deadline = Instant::now() + SPAWN_CONNECT_TIMEOUT;
    while Instant::now() < deadline {
        if probe_handshake().await {
            return Ok(());
        }
        if !noted_ready {
            eprintln!("[tenex-edge] waiting for daemon to answer RPCs...");
            noted_ready = true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("daemon did not answer handshakes within {SPAWN_CONNECT_TIMEOUT:?}")
}

/// Liveness probe: can we complete the daemon hello/welcome handshake?
async fn probe_handshake() -> bool {
    let Ok(Ok(stream)) =
        tokio::time::timeout(HANDSHAKE_PROBE_TIMEOUT, UnixStream::connect(socket_path())).await
    else {
        return false;
    };
    let (rh, wh) = stream.into_split();
    let mut reader = BufReader::new(rh);
    let mut writer = wh;
    if write_line(
        &mut writer,
        &Hello {
            protocol: protocol_version(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .await
    .is_err()
    {
        return false;
    }
    let Ok(Ok(Some(_welcome))) =
        tokio::time::timeout(HANDSHAKE_PROBE_TIMEOUT, read_line::<Welcome>(&mut reader)).await
    else {
        return false;
    };
    true
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
        .truncate(false)
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
