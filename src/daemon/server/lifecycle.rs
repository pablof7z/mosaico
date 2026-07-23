use super::*;
use crate::reconcile::StatusReconciler;
mod auth_restore;
mod pending_writes;
mod roster_bootstrap;

pub async fn run() -> Result<()> {
    let storage = crate::daemon::storage_paths::StoragePaths::current();
    config::ensure_dir(&storage.mosaico_home)?;
    crate::logging::init_daemon_logging(&storage.daemon_log_path)?;
    let lock = match StartupLock::try_acquire()? {
        Some(l) => l,
        None => {
            tracing::info!("another daemon already running; exiting");
            return Ok(());
        }
    };
    let listener = bind_socket()?;
    tracing::info!(
        mosaico_home = %storage.mosaico_home.display(),
        config = %storage.config_path.display(),
        socket = %storage.socket_path.display(),
        state_db = %storage.state_db_path.display(),
        nmp_store = %storage.nmp_store_path.display(),
        daemon_log = %storage.daemon_log_path.display(),
        lock = %storage.lock_path.display(),
        mosaico_home_set = storage.mosaico_home_set,
        mosaico_home_is_default = storage.mosaico_home_is_default,
        "daemon storage paths"
    );
    tracing::info!(socket = %socket_path().display(), "daemon listening");
    let (cfg, backend_keys) = auth_restore::load_backend()?;
    let host = cfg.host.clone();
    let owners = cfg.whitelisted_pubkeys.clone();
    // The narrow direct client handles one-shot reads and doctor probes. Its
    // AUTH identity is irrelevant to delivery (verified:
    // an A-authed connection receives events p-tagged to B), so authenticate
    // with the backend's own key (`mosaicoPrivateKey`) rather than minting a
    // separate identity — a fresh keystore file would land in the same
    // `agents/` directory as real agents and leak into `who`/invite listings
    // as a phantom agent.
    // The indexer is a direct-client target for one-shot lookups. Every product
    // write, including kind:0 copies to this indexer, is routed durably by NMP.
    let indexer = (!cfg.relays.contains(&cfg.indexer_relay)).then_some(cfg.indexer_relay.as_str());
    let transport = Arc::new(
        Transport::connect_with_indexer(&cfg.relays, indexer, backend_keys.clone())
            .await
            .context("daemon relay connect")?,
    );
    tracing::info!(
        relays = ?cfg.relays,
        indexer = ?cfg.indexer_relay,
        "relay pool connected"
    );
    let store = Arc::new(Mutex::new(Store::open(&store_path())?));
    let nmp = Arc::new(crate::nmp_host::NmpHost::open(
        &cfg.relays,
        Some(&cfg.indexer_relay),
        Some(&storage.nmp_store_path),
        &backend_keys,
    )?);
    let provider = Arc::new(Nip29Provider::new(
        transport.clone(),
        nmp.clone(),
        store.clone(),
        cfg.management_nsec().cloned(),
        cfg.user_nsec().cloned(),
        cfg.whitelisted_pubkeys.clone(),
    ));
    let presence_publisher =
        crate::presence_publisher::PresencePublisher::spawn(provider.clone(), store.clone());
    let state = Arc::new(DaemonState {
        store,
        transport,
        provider,
        nmp,
        cfg,
        host,
        owners,
        agent_config: AgentConfigState::new(),
        catalog: CatalogState::new(),
        runtime: SessionRuntimeState::new(),
        subscriptions: SubscriptionState::new(),
        reconcilers: ReconcilerState::new(
            StatusReconciler::for_ttl(presence_lease_ttl()),
            presence_publisher,
        ),
        connections: ConnectionState::new(),
        dedup: DedupState::new(),
        standing_sync: tokio::sync::Mutex::new(()),
        mcp_actor_sync: tokio::sync::Mutex::new(()),
    });
    auth_restore::restore(&state).context("restoring NIP-42 identities")?;
    pending_writes::spawn(&storage.state_db_path, &state.nmp);
    // These tolerate a not-yet-connected relay, so they start now.
    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    super::managed_lifecycle::spawn(state.clone());

    // Freeze restart-recovery ownership before accepting RPCs. Relay warmup is
    // intentionally asynchronous, but a session admitted through the accept
    // loop after this point belongs to this daemon incarnation and must never be
    // mistaken for an orphan left by the previous process.
    let startup_sessions = state.with_store(|store| store.list_running_sessions())?;

    let accept_state = state.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let st = accept_state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_connection(st, stream).await {
                            tracing::debug!(error = %e, "connection error");
                        }
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "accept loop error");
                    break;
                }
            }
        }
    });

    // Relay startup runs off the accept path, so store-only RPCs respond immediately.
    // Warm the narrow direct connection before fetches.
    let relay_state = state.clone();
    tokio::spawn(async move {
        relay_state.transport.warmup().await;
        tracing::info!("relay warmup complete; opening subscriptions");
        super::agent_discovery::start_monitor(relay_state.clone());

        // Publish the backend kind:0 with its managed-agent tags. It is intentionally
        // absent from hosted state so the echo never reaches `who` or agent context.
        // Failure is best-effort and deferred to the next roster change or restart.
        super::agent_roster::publish_backend_profile(&relay_state).await;

        // Proactively warm the profiles we already know we care about — the human
        // operator(s) and every persisted local session pubkey — so the first awareness
        // renders them by name instead of raw hex. Members we learn about later are
        // warmed as their 3900x events arrive (see `warm_profiles` in the demux).
        {
            let mut known = relay_state.owners.clone();
            known.extend(
                relay_state.with_store(|s| s.list_local_session_pubkeys().unwrap_or_default()),
            );
            warm_profiles(&relay_state, known);
        }

        roster_bootstrap::publish_startup_roster(&relay_state).await;

        // Seed the daemon-lifetime kind:9000 discovery observation plus the
        // refcounted per-entity #h / #p / group-state observations. No kind:0 is
        // observed live — a
        // put-user p-tag triggers an on-demand profile fetch in the demux.
        if let Err(e) = sync_subscriptions(&relay_state).await {
            tracing::warn!(error = %e, "initial subscription sync failed");
        }

        // Revive sessions a previous daemon left behind and reconcile their NMP
        // observations.
        reconcile_sessions(&relay_state, startup_sessions).await;
        // Re-adopted sessions may already have inbox rows from before the daemon
        // restart. Session start rings these rows, but reconciliation does not;
        // ring once here so a restart cannot leave messages pending until an
        // unrelated later relay event arrives.
        crate::session_host::ring_doorbells(relay_state.clone());
    });

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    tokio::select! {
        _ = state.connections.shutdown.notified() => {}
        _ = async { match &mut sigterm { Some(s) => { s.recv().await; }, None => std::future::pending().await } } => {}
    }
    tracing::info!("daemon shutting down");
    accept.abort();
    shutdown_rpc_sessions(&state).await;
    cleanup();
    state.nmp.shutdown();
    state.transport.shutdown().await;
    drop(lock);
    Ok(())
}

async fn shutdown_rpc_sessions(state: &Arc<DaemonState>) {
    for (kind, endpoint, confirmation) in
        crate::session_host::transport::acp::shutdown_owned_sessions().await
    {
        match confirmation {
            Ok(()) => {
                let session = state
                    .with_store(|store| {
                        store.session_for_runtime_locator(kind.locator_kind(), &endpoint)
                    })
                    .ok()
                    .flatten();
                if let Some(session) = session {
                    if let Err(error) = state.with_store(|store| {
                        store.mark_runtime_stopped_if_generation(
                            &session.pubkey,
                            session.runtime_generation,
                            crate::state::StopReason::Superseded,
                            crate::util::now_secs(),
                        )
                    }) {
                        tracing::error!(%endpoint, %error, "RPC shutdown state update failed");
                    }
                }
            }
            Err(error) => {
                tracing::error!(%endpoint, %error, "RPC process-group shutdown failed");
            }
        }
    }
}

pub(in crate::daemon::server) fn bind_socket() -> Result<UnixListener> {
    let sock = socket_path();
    if sock.exists() {
        let _ = std::fs::remove_file(&sock);
    }
    UnixListener::bind(&sock).with_context(|| format!("binding {}", sock.display()))
}

pub(in crate::daemon::server) fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
    // Do NOT remove the lock file here — deleting it while the flock is still
    // held lets a racing spawner open a *new* file (different inode) and acquire
    // an independent lock, causing two daemons to overlap and fight over state.db.
    // The lock is implicitly released when the process exits and the fd is closed.
}

pub(in crate::daemon::server) async fn serve_connection(
    state: Arc<DaemonState>,
    stream: UnixStream,
) -> Result<()> {
    let (rh, wh) = stream.into_split();
    let mut reader = BufReader::new(rh);
    let mut writer = wh;

    let mut first = String::new();
    if reader.read_line(&mut first).await? == 0 {
        return Ok(());
    }
    let hello: Hello = serde_json::from_str(first.trim_end()).context("parsing hello")?;
    write_json(
        &mut writer,
        &Welcome {
            protocol: protocol_version(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .await?;

    if hello.protocol > protocol_version() {
        let mut line = String::new();
        if reader.read_line(&mut line).await? > 0
            && serde_json::from_str::<PleaseExit>(line.trim_end()).is_ok()
        {
            tracing::info!(
                client_protocol = hello.protocol,
                "newer client; restarting daemon for re-exec"
            );
            state.connections.shutdown.notify_waiters();
        }
        let _ = write_json(
            &mut writer,
            &Response::err(0, ERR_PROTOCOL_SKEW, "daemon exiting for re-exec"),
        )
        .await;
        return Ok(());
    }

    {
        *state.connections.open_clients.lock().unwrap() += 1;
    }
    let _guard = ClientGuard(state.clone());

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        let req: Request = match serde_json::from_str(line) {
            Ok(r) => r,
            Err(e) => {
                write_json(&mut writer, &Response::err(0, "bad_request", e.to_string())).await?;
                continue;
            }
        };
        match req.method.as_str() {
            "tail" => {
                handle_tail(&state, req.id, &req.params, &mut writer).await?;
                break; // tail owns the connection until the client disconnects
            }
            "channel_read" => {
                if let Err(e) = handle_channel_read(&state, req.id, &req.params, &mut writer).await
                {
                    write_json(
                        &mut writer,
                        &Response::err(req.id, "channel_read_failed", format!("{e:#}")),
                    )
                    .await?;
                }
                break;
            }
            "session_start" => {
                handle_session_start(&state, req.id, &req.params, &mut writer).await?;
            }
            _ => {
                let resp = dispatch(&state, &req).await;
                write_json(&mut writer, &resp).await?;
            }
        }
    }
    Ok(())
}

pub(in crate::daemon::server) struct ClientGuard(pub(in crate::daemon::server) Arc<DaemonState>);
impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut n = self.0.connections.open_clients.lock().unwrap();
        *n = n.saturating_sub(1);
    }
}

pub(in crate::daemon::server) async fn write_json<T: serde::Serialize, W: AsyncWriteExt + Unpin>(
    w: &mut W,
    v: &T,
) -> Result<()> {
    let mut line = serde_json::to_string(v)?;
    line.push('\n');
    w.write_all(line.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

#[derive(Clone)]
pub(in crate::daemon::server) struct InitProgress {
    started: Instant,
    tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>,
}

impl InitProgress {
    fn new(tx: tokio::sync::mpsc::UnboundedSender<serde_json::Value>) -> Self {
        Self {
            started: Instant::now(),
            tx,
        }
    }

    pub(in crate::daemon::server) fn emit(&self, phase: &str, message: impl Into<String>) {
        let _ = self.tx.send(serde_json::json!({
            "kind": "init_progress",
            "phase": phase,
            "message": message.into(),
            "elapsed_ms": self.started.elapsed().as_millis() as u64,
        }));
    }
}

pub(in crate::daemon::server) async fn handle_session_start<W: AsyncWriteExt + Unpin>(
    state: &Arc<DaemonState>,
    id: u64,
    params: &serde_json::Value,
    writer: &mut W,
) -> Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let progress = InitProgress::new(tx);
    let fut = rpc_session_start(state, params, Some(progress));
    tokio::pin!(fut);

    let result = loop {
        tokio::select! {
            Some(item) = rx.recv() => {
                write_json(writer, &Response::item(id, item)).await?;
            }
            result = &mut fut => break result,
        }
    };

    while let Ok(item) = rx.try_recv() {
        write_json(writer, &Response::item(id, item)).await?;
    }

    let resp = match result {
        Ok(v) => Response::ok(id, v),
        Err(e) => Response::err(id, "rpc_error", format!("{e:#}")),
    };
    write_json(writer, &resp).await
}
