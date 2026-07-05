use super::super::spawn::spawn_detached_daemon;
use super::*;

// ── spawn-if-absent (race-safe via flock) ────────────────────────────────────

/// Ensure a daemon is listening. Under the startup lock: re-check the socket,
/// reclaim a stale one, then spawn a detached daemon and poll-connect.
pub(super) async fn spawn_daemon_if_absent() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;

    let mut noted_wait = false;
    let mut spawned_child: Option<std::process::Child> = None;
    let wait_deadline = Instant::now() + DAEMON_STARTUP_TIMEOUT;
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
            spawned_child = Some(spawn_detached_daemon()?);
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

    // Watch a child *we* just spawned: a daemon that dies immediately (e.g.
    // missing config.json) is reported right away instead of waiting out the
    // full timeout for a generic "did not answer handshakes".
    let mut noted_ready = false;
    let deadline = Instant::now() + DAEMON_STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if probe_handshake().await {
            return Ok(());
        }
        if let Some(child) = spawned_child.as_mut() {
            match crate::daemon::poll_spawned_child(child) {
                crate::daemon::SpawnedChildStatus::LostRace => spawned_child = None,
                crate::daemon::SpawnedChildStatus::Crashed(msg) => bail!("{msg}"),
                crate::daemon::SpawnedChildStatus::Running => {}
            }
        }
        if !noted_ready {
            eprintln!("[tenex-edge] waiting for daemon to answer RPCs...");
            noted_ready = true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    bail!("daemon did not answer handshakes within {DAEMON_STARTUP_TIMEOUT:?}")
}

/// Liveness probe: can we complete the daemon hello/welcome handshake?
async fn probe_handshake() -> bool {
    let Ok(Ok(stream)) = tokio::time::timeout(
        DAEMON_HANDSHAKE_IO_TIMEOUT,
        UnixStream::connect(socket_path()),
    )
    .await
    else {
        return false;
    };
    let (rh, wh) = stream.into_split();
    let mut reader = BufReader::new(rh);
    let mut writer = wh;
    if write_line(&mut writer, &client_hello()).await.is_err() {
        return false;
    }
    let Ok(Ok(Some(_welcome))) = tokio::time::timeout(
        DAEMON_HANDSHAKE_IO_TIMEOUT,
        read_line::<Welcome>(&mut reader),
    )
    .await
    else {
        return false;
    };
    true
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
