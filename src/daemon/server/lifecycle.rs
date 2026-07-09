use super::*;
use crate::reconcile::StatusReconciler;

mod roster_bootstrap;

pub async fn run() -> Result<()> {
    let storage = crate::daemon::storage_paths::StoragePaths::current();
    config::ensure_dir(&storage.edge_home)?;
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
        edge_home = %storage.edge_home.display(),
        config = %storage.config_path.display(),
        socket = %storage.socket_path.display(),
        state_db = %storage.state_db_path.display(),
        daemon_log = %storage.daemon_log_path.display(),
        lock = %storage.lock_path.display(),
        tenex_edge_home_set = storage.tenex_edge_home_set,
        edge_home_is_default = storage.edge_home_is_default,
        "daemon storage paths"
    );
    tracing::info!(socket = %socket_path().display(), "daemon listening");
    let cfg = Config::load().context("loading config")?;
    let host = cfg.host.clone();
    let owners = cfg.whitelisted_pubkeys.clone();
    let started_at = now_secs();
    // One relay connection. AUTH identity is irrelevant to delivery (verified:
    // an A-authed connection receives events p-tagged to B), so authenticate
    // with the backend's own key (`tenexPrivateKey`) rather than minting a
    // separate identity — a fresh keystore file would land in the same
    // `agents/` directory as real agents and leak into `who`/invite listings
    // as a phantom agent.
    let auth_keys = cfg
        .backend_nsec()
        .and_then(|nsec| Keys::parse(nsec).ok())
        .unwrap_or_else(Keys::generate);
    // The indexer relay is added with full READ+WRITE flags (see
    // `Transport::write_relay_urls` doc) and targeted explicitly for kind:0
    // profile publishes via `publish_event_to`. It MUST stay OUT of
    // `write_relay_urls` (the broadcast target for every other publish): the
    // indexer (purplepag.es) rejects all NIP-29 kinds ("blocked: kind 9000 is
    // not allowed"), and that rejection would pollute `assert_relay_accepted`'s
    // joined-reason verdict whenever the main relay also returned a benign
    // rejection — turning recoverable NIP-29 states into permanent
    // `ChannelGate::Degraded`.
    let indexer = if cfg.relays.contains(&cfg.indexer_relay) {
        None
    } else {
        Some(cfg.indexer_relay.as_str())
    };
    let transport = Arc::new(
        Transport::connect_with_indexer(&cfg.relays, indexer, auth_keys)
            .await
            .context("daemon relay connect")?,
    );
    tracing::info!(
        relays = ?cfg.relays,
        indexer = ?cfg.indexer_relay,
        "relay pool connected"
    );
    let store = Arc::new(Mutex::new(Store::open(&store_path())?));
    let provider = Arc::new(Nip29Provider::new(
        transport.clone(),
        store.clone(),
        cfg.management_nsec().cloned(),
        cfg.user_nsec().cloned(),
        cfg.whitelisted_pubkeys.clone(),
        &cfg.relays, // provider_instance hashes main relays only, not indexer
    ));
    let state = Arc::new(DaemonState {
        store,
        transport,
        provider,
        cfg,
        host,
        started_at,
        owners,
        hosted: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        subscribed_root_channels: Mutex::new(Vec::new()),
        subs: Mutex::new(crate::reconcile::SubscriptionReconciler::new().expect("subs")),
        status: Arc::new(Mutex::new(StatusReconciler::for_ttl(status_ttl_duration()))),
        delivery: Mutex::new(crate::reconcile::DeliveryReconciler::new()),
        turn_lifecycle: Mutex::new(crate::reconcile::TurnLifecycleReconciler::new()),
        cursor: Mutex::new(crate::reconcile::CursorReconciler::new()),
        session_start: Mutex::new(crate::reconcile::SessionStartReconciler::new()),
        session_watch: Mutex::new(crate::reconcile::Reconciler::new().expect("session_watch")),
        outbox: Arc::new(Mutex::new(crate::reconcile::OutboxReconciler::new())),
        hook_contexts: Mutex::new(HashMap::new()),
        tail_tx: tokio::sync::broadcast::channel(512).0,
        open_clients: Mutex::new(0),
        shutdown: Notify::new(),
        peer_sessions: Mutex::new(HashMap::new()),
        seen_events: Mutex::new((
            std::collections::HashSet::new(),
            std::collections::VecDeque::new(),
        )),
        seen_profiles: Mutex::new(std::collections::HashSet::new()),
        warming: Mutex::new(std::collections::HashSet::new()),
        last_status: Mutex::new(HashMap::new()),
        outbox_notify: Notify::new(),
        session_keys: Mutex::new(HashMap::new()),
    });

    // These tolerate a not-yet-connected relay, so they start now.
    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    spawn_trellis_oracle_sampler(state.clone());
    spawn_outbox_drainer(state.clone());

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

    // Relay startup runs off the accept path; store-only RPCs respond immediately. We warm up
    // the connection (await connectivity + NIP-42 auth) BEFORE opening any
    // subscription — a REQ opened pre-auth on an auth-gated relay never delivers.
    let relay_state = state.clone();
    tokio::spawn(async move {
        relay_state.transport.warmup().await;
        tracing::info!("relay warmup complete; opening subscriptions");

        // Publish the backend's own kind:0 so it is identifiable on the relay by
        // Nostr clients. Best-effort: failure deferred to next restart.
        // Intentionally NOT stored in the hosted set — the echo must NOT appear in
        // `who` or be injected into agent turn-context.
        if let Some(backend_keys) = relay_state.provider.management_keys() {
            let profile = crate::domain::Profile::backend_named(
                backend_keys.public_key().to_hex(),
                format!("{} (tenex-edge)", relay_state.host),
                relay_state.host.clone(),
                relay_state.owners.clone(),
            );
            let ev = crate::domain::DomainEvent::Profile(profile);
            let _ = relay_state.provider.publish(&ev, &backend_keys).await;
        }

        // Proactively warm the profiles we already know we care about — the human
        // operator(s) and every durable agent identity — so the very first `who`
        // renders them by name instead of raw hex. Members we learn about later are
        // warmed as their 3900x events arrive (see `warm_profiles` in the demux).
        {
            let mut known = relay_state.owners.clone();
            known.extend(relay_state.with_store(|s| s.list_identity_pubkeys().unwrap_or_default()));
            warm_profiles(&relay_state, known);
        }

        roster_bootstrap::publish_startup_roster(&relay_state).await;
        membership_cleanup::cleanup_dead_local_sessions(&relay_state);
        roster_bootstrap::seed_spawn_on_mention_coverage(&relay_state);

        // Seed the three stable aggregate REQs (#h / #p / group-state) once. This
        // replaces both the per-member-group subscription loop and the standalone
        // backend orchestration REQ (the backend pubkey is now in the #p aggregate).
        // No kind:0 is subscribed — profiles resolve on demand via Transport::fetch.
        if let Err(e) = resubscribe(&relay_state).await {
            tracing::warn!(error = %e, "initial resubscribe failed");
        }

        // Revive sessions a previous daemon left behind + (re)open their channel
        // subscriptions. Subscriptions go out post-auth.
        reconcile_sessions(&relay_state).await;
    });

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    tokio::select! {
        _ = state.shutdown.notified() => {}
        _ = async { match &mut sigterm { Some(s) => { s.recv().await; }, None => std::future::pending().await } } => {}
    }
    tracing::info!("daemon shutting down");
    accept.abort();
    cleanup();
    state.transport.shutdown().await;
    drop(lock);
    Ok(())
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

// ── connection handling ──────────────────────────────────────────────────────

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
            state.shutdown.notify_waiters();
        }
        let _ = write_json(
            &mut writer,
            &Response::err(0, ERR_PROTOCOL_SKEW, "daemon exiting for re-exec"),
        )
        .await;
        return Ok(());
    }

    {
        *state.open_clients.lock().unwrap() += 1;
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
            "chat_read" => {
                if let Err(e) = handle_chat_read(&state, req.id, &req.params, &mut writer).await {
                    write_json(
                        &mut writer,
                        &Response::err(req.id, "chat_read_failed", format!("{e:#}")),
                    )
                    .await?;
                }
                break; // chat_read may own the connection for --live
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
        let mut n = self.0.open_clients.lock().unwrap();
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

// ── dispatch (one-shot verbs) ────────────────────────────────────────────────
