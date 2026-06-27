use super::*;

pub async fn run() -> Result<()> {
    config::ensure_dir(&config::edge_home())?;

    let lock = match StartupLock::try_acquire()? {
        Some(l) => l,
        None => {
            eprintln!("[daemon] another daemon already running; exiting");
            return Ok(());
        }
    };
    let listener = bind_socket()?;
    eprintln!("[daemon] listening on {}", socket_path().display());

    let cfg = Config::load().context("loading config")?;
    let host = cfg.host.clone();
    let owners = cfg.whitelisted_pubkeys.clone();

    // One relay connection. AUTH identity is irrelevant to delivery (verified:
    // an A-authed connection receives events p-tagged to B), so authenticate
    // with a stable daemon key and sign each event with its true author.
    let auth_keys = identity::load_or_create(&config::edge_home(), "tenex-edge-daemon", now_secs())
        .map(|i| i.keys)
        .unwrap_or_else(|_| Keys::generate());
    // Include the indexer relay in the transport pool so kind:0 publishes reach
    // it and kind:0 subscriptions also query it for profile discovery. Deduped
    // in case someone lists purplepag.es in their main relays too.
    let transport_relays: Vec<String> = {
        let mut v = cfg.relays.clone();
        if !v.iter().any(|r| r == &cfg.indexer_relay) {
            v.push(cfg.indexer_relay.clone());
        }
        v
    };
    let transport = Arc::new(
        Transport::connect(&transport_relays, auth_keys)
            .await
            .context("daemon relay connect")?,
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
    // Backend identity: pubkey of `tenexPrivateKey` (no `userNsec` fallback —
    // the operator key is a human identity, not a backend identity). Used as a
    // copied admin on every group we create and as the orchestration listener's
    // `add`-tag matcher.
    let backend_pubkey: Option<String> = cfg
        .backend_nsec()
        .and_then(|n| Keys::parse(n).ok())
        .map(|k| k.public_key().to_hex());

    let state = Arc::new(DaemonState {
        store,
        transport,
        provider,
        cfg,
        host,
        owners,
        hosted: Mutex::new(HashMap::new()),
        sessions: Mutex::new(HashMap::new()),
        subscribed_projects: Mutex::new(Vec::new()),
        subscriptions: Mutex::new(crate::fabric::subscriptions::SubscriptionRegistry::new()),
        tail_tx: tokio::sync::broadcast::channel(512).0,
        open_clients: Mutex::new(0),
        liveness_changed: Notify::new(),
        shutdown: Notify::new(),
        peer_sessions: Mutex::new(HashMap::new()),
        seen_events: Mutex::new((
            std::collections::HashSet::new(),
            std::collections::VecDeque::new(),
        )),
        seen_profiles: Mutex::new(std::collections::HashSet::new()),
        last_status: Mutex::new(HashMap::new()),
        status_outbox_notify: Notify::new(),
        session_keys: Mutex::new(HashMap::new()),
        session_signers: Mutex::new(HashMap::new()),
        backend_pubkey,
    });

    // Idempotent read-model backfill: populate canonical `projects` + `membership`
    // tables from legacy data so readers have a consistent origin on every start.
    // Best-effort: a backfill error must not prevent startup.
    {
        let pi = state.provider.provider_instance.clone();
        state.with_store(|s| {
            s.backfill_nip29_origins(&pi, now_secs()).ok();
        });
    }

    // These tolerate a not-yet-connected relay (demux just waits for events;
    // publishers/subscribers are best-effort and queue), so they start now.
    spawn_demux(state.clone());
    spawn_pruner(state.clone());
    spawn_idle_watcher(state.clone());
    spawn_status_outbox_drainer(state.clone());
    spawn_status_heartbeat_publisher(state.clone());
    spawn_retry_drainer(state.clone());

    let accept_state = state.clone();
    let accept = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let st = accept_state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = serve_connection(st, stream).await {
                            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                                eprintln!("[daemon] connection error: {e:#}");
                            }
                        }
                    });
                }
                Err(e) => {
                    eprintln!("[daemon] accept error: {e}");
                    break;
                }
            }
        }
    });

    // Relay-DEPENDENT startup runs in the background, OFF the accept path, so the
    // daemon serves store-only RPCs (`who`, `tmux`, chat/inbox reads, statusline,
    // whoami) immediately even when the relay is slow or unreachable. We warm up
    // the connection (await connectivity + NIP-42 auth) BEFORE opening any
    // subscription — a REQ opened pre-auth on an auth-gated relay never delivers.
    let relay_state = state.clone();
    tokio::spawn(async move {
        relay_state.transport.warmup().await;

        // Publish the backend's own kind:0 so it is identifiable on the relay by
        // Nostr clients. Best-effort: failure deferred to next restart.
        // Intentionally NOT stored in the hosted set — the echo must NOT appear in
        // `who` or be injected into agent turn-context.
        if let Some(nsec) = relay_state.cfg.backend_nsec() {
            if let Ok(backend_keys) = nostr_sdk::prelude::Keys::parse(nsec) {
                let name = format!("{} (tenex-edge)", relay_state.host);
                let ev = crate::domain::DomainEvent::Profile(crate::domain::Profile {
                    agent: crate::domain::AgentRef::new(backend_keys.public_key().to_hex(), name),
                    host: relay_state.host.clone(),
                    owners: relay_state.owners.clone(),
                    is_backend: true,
                });
                let _ = relay_state.provider.publish(&ev, &backend_keys).await;
            }
        }

        // Discover groups where local agents are already members so kind:9 chat
        // arrives even when no session is alive (spawn-on-mention path), and record
        // them in `subscribed_projects`. We DON'T open a REQ per group here — the
        // single `resubscribe` below folds all of them (plus owned groups and the
        // backend identity) into the three stable aggregate REQs. New memberships
        // discovered from 39002 events extend coverage via `ensure_subscription`.
        {
            let edge = crate::config::edge_home();
            let local_pks: Vec<String> = crate::identity::list_local_pubkeys(&edge);
            let member_groups: Vec<String> = relay_state.with_store(|s| {
                let mut groups = Vec::new();
                for pk in &local_pks {
                    if let Ok(gs) = s.list_groups_for_member(pk) {
                        groups.extend(gs);
                    }
                }
                groups.sort_unstable();
                groups.dedup();
                groups
            });
            {
                let mut projs = relay_state.subscribed_projects.lock().unwrap();
                for group in &member_groups {
                    if !projs.iter().any(|p| p == group) {
                        projs.push(group.clone());
                    }
                }
            }
            eprintln!(
                "[daemon] spawn-on-mention: {} local agents, {} member groups tracked",
                local_pks.len(),
                member_groups.len()
            );
        }

        // Seed the three stable aggregate REQs (#h / #p / group-state) once. This
        // replaces both the per-member-group subscription loop and the standalone
        // backend orchestration REQ (the backend pubkey is now in the #p aggregate).
        // No kind:0 is subscribed — profiles resolve on demand via Transport::fetch.
        if let Err(e) = resubscribe(&relay_state).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] initial resubscribe failed: {e:#}");
            }
        }

        // Revive sessions a previous daemon left behind + (re)open their project
        // subscriptions. Subscriptions go out post-auth.
        reconcile_sessions(&relay_state).await;
    });

    let mut sigterm =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).ok();
    tokio::select! {
        _ = state.shutdown.notified() => {}
        _ = async { match &mut sigterm { Some(s) => { s.recv().await; }, None => std::future::pending().await } } => {}
    }
    eprintln!("[daemon] shutting down");
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
    let _ = std::fs::remove_file(lock_path());
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
            eprintln!(
                "[daemon] newer client (protocol {}); exiting for re-exec",
                hello.protocol
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
        state.liveness_changed.notify_waiters();
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
                handle_chat_read(&state, req.id, &req.params, &mut writer).await?;
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
        self.0.liveness_changed.notify_waiters();
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
