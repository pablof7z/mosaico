use super::*;

mod session_watch;

pub(in crate::daemon::server) async fn spawn_session(
    state: &Arc<DaemonState>,
    params: EngineParams,
) -> Result<()> {
    let session_id = params.session_id.clone();
    let pubkey = params.identity.pubkey.clone();
    let project = params.project.clone();
    let watch_pid = params.watch_pid;

    tracing::info!(
        agent = %params.identity.slug,
        channel = %project,
        session = %session_id,
        "spawning session engine"
    );

    state.hosted.lock().unwrap().insert(
        pubkey.clone(),
        HostedAgent {
            keys: params.keys.clone(),
        },
    );
    let st = state.clone();
    let project_for_sub = project.clone();
    tokio::spawn(async move {
        if let Err(e) = ensure_subscription(&st, &project_for_sub).await {
            tracing::warn!(
                channel = %project_for_sub,
                error = %e,
                "session subscription setup failed"
            );
        }
    });

    let cancel = Arc::new(Notify::new());
    state.sessions.lock().unwrap().insert(
        session_id.clone(),
        SessionHandle {
            cancel: cancel.clone(),
        },
    );
    session_watch::started(
        state,
        &session_id,
        &project,
        &pubkey,
        watch_pid,
        "spawn-session",
    );

    let st = state.clone();
    let sid = session_id.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    let status = state.status.clone();
    let outbox = state.outbox.clone();
    tokio::spawn(async move {
        let res =
            runtime::run_session_in_daemon(params, provider, store, cancel, status, outbox).await;
        if let Err(e) = res {
            tracing::warn!(session = %sid, error = %e, "session task exited with error");
        }
        membership_cleanup::remove_session_memberships(&st, &sid, "engine-exit");
        st.release_session_signer(&sid);
        session_watch::exited(&st, &sid, watch_pid, "engine-exit");
        // Mark the bound identity dead but keep the row for resume (issue #47).
        st.with_store(|s| {
            if let Err(e) = s.mark_identity_dead_for_session(&sid) {
                tracing::error!(
                    session = %sid,
                    error = %e,
                    "engine exit: failed to mark identity dead; `who` may show a ghost"
                );
            }
        });
        st.sessions.lock().unwrap().remove(&sid);
        prune_hosted(&st);
        tracing::info!(session = %sid, "session engine exited");
    });
    Ok(())
}

pub(in crate::daemon::server) fn prune_hosted(state: &Arc<DaemonState>) {
    let live: std::collections::HashSet<String> = state
        .with_store(|s| s.list_alive_sessions().unwrap_or_default())
        .into_iter()
        .map(|r| r.agent_pubkey)
        .collect();
    state
        .hosted
        .lock()
        .unwrap()
        .retain(|pk, _| live.contains(pk));
}

pub(in crate::daemon::server) fn cancel_session(
    state: &Arc<DaemonState>,
    session_id: &str,
) -> bool {
    if let Some(h) = state.sessions.lock().unwrap().get(session_id) {
        h.cancel.notify_waiters();
        true
    } else {
        false
    }
}

/// Revive sessions a previous daemon left behind (skew re-exec / crash),
/// rebuilding from the `sessions` table. For each ALIVE session: respawn the
/// engine task if its `child_pid` is still alive, else mark it dead (so
/// `who`/presence don't lie after a restart) and crash-GC its ordinal member.
pub(in crate::daemon::server) async fn reconcile_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let snaps = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
    tracing::info!(
        session_count = snaps.len(),
        "reconciling sessions from previous daemon instance"
    );
    for snap in snaps {
        let session_id = snap.session_id.clone();
        let pid_ok = snap.child_pid.map(pid_alive).unwrap_or(false);
        if !pid_ok {
            tracing::warn!(
                session = %session_id,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                pid = ?snap.child_pid,
                "session process dead; marking dead and GC-ing ordinal membership"
            );
            state.with_store(|s| {
                if let Err(e) = s.mark_dead(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "reconcile GC: failed to mark dead session dead; ghost-alive row may remain");
                }
                if let Err(e) = s.mark_identity_dead_for_session(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "reconcile GC: failed to mark identity dead for dead session");
                }
            });
            session_watch::exited_at(
                state,
                &session_id,
                snap.child_pid,
                now,
                "reconcile-dead-pid",
            );
            membership_cleanup::remove_session_memberships(
                state,
                &session_id,
                "reconcile-dead-pid",
            );
            continue;
        }
        tracing::info!(
            session = %session_id,
            agent = %snap.agent_slug,
            channel = %snap.channel_h,
            pid = ?snap.child_pid,
            "reviving session from previous daemon instance"
        );
        let minted = match mint_session_identity(
            state,
            &session_id,
            &snap.agent_slug,
            &snap.channel_h,
            &snap.resume_id,
        ) {
            Ok(minted) => minted,
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "identity mint failed during reconcile; skipping session");
                continue;
            }
        };

        // Re-establish membership + the group-state subscription through the one
        // channel-provisioning primitive. The scope may be a top-level project
        // (root channel) or a subgroup; its stored parent (if any) is the
        // readiness gate's parent_hint. Idempotent: the relay_channel* cache
        // persists across restarts, so already-ready channels fast-path.
        let parent_hint = state
            .with_store(|s| s.channel_parent(&snap.channel_h).ok().flatten())
            .filter(|p| !p.is_empty());
        let gate = state
            .provider
            .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
                channel: &snap.channel_h,
                expect_member: &minted.identity.pubkey,
                parent_hint: parent_hint.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            })
            .await;
        // `Degraded` means the channel was NOT verified ready on the relay.
        // Respawning the engine against an unverified channel would publish into a
        // phantom scope, so quarantine the session (mark it dead) instead of
        // reviving it — loudly, since a revival that silently degrades hides relay
        // breakage behind a "running" session.
        if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
            tracing::error!(
                session = %session_id,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                "channel not verified ready on reconcile; quarantining session instead of respawning"
            );
            state.with_store(|s| {
                if let Err(e) = s.mark_dead(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "failed to mark session dead during reconcile quarantine");
                }
                if let Err(e) = s.mark_identity_dead_for_session(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "failed to mark identity dead during reconcile quarantine");
                }
            });
            state.release_session_signer(&session_id);
            continue;
        }

        // Rebind the row to the minted session pubkey so mention routing keys on
        // this session's real identity.
        state.with_store(|s| {
            if let Err(e) = s.set_session_agent_pubkey(&session_id, &minted.identity.pubkey) {
                tracing::error!(
                    session = %session_id,
                    pubkey = %minted.identity.pubkey,
                    error = %e,
                    "reconcile: failed to rebind session to minted pubkey; mention routing may miss"
                );
            }
        });

        if let Err(e) = ensure_subscription(state, &snap.channel_h).await {
            tracing::warn!(channel = %snap.channel_h, error = %e, "ensure_subscription failed during reconcile");
        }
        let ep = engine_params_for(
            &state.cfg,
            minted.identity.clone(),
            minted.keys.clone(),
            &session_id,
            &snap.channel_h,
            "",
            snap.child_pid,
        );
        if let Err(e) = spawn_session(state, ep).await {
            tracing::error!(
                session = %session_id,
                error = %e,
                "reconcile: failed to respawn session engine; releasing selected signer"
            );
            state.release_session_signer(&session_id);
            state.with_store(|s| {
                if let Err(e) = s.mark_dead(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "failed to mark session dead after reconcile spawn failure");
                }
                if let Err(e) = s.mark_identity_dead_for_session(&session_id) {
                    tracing::error!(session = %session_id, error = %e, "failed to mark identity dead after reconcile spawn failure");
                }
            });
        }
    }
    // Any registration/end transitions above enqueued publishes.
    state.outbox_notify.notify_waiters();
}

#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn engine_params_for(
    cfg: &Config,
    // The session's read-side identity: pubkey, agent slug, codename.
    identity: crate::identity::SessionIdentity,
    // The session's own minted signing keypair.
    keys: Keys,
    session_id: &str,
    project: &str,
    rel_cwd: &str,
    watch_pid: Option<i32>,
) -> EngineParams {
    EngineParams {
        identity,
        keys,
        project: project.to_string(),
        session_id: session_id.to_string(),
        host: cfg.host.clone(),
        rel_cwd: rel_cwd.to_string(),
        owners: cfg.whitelisted_pubkeys.clone(),
        relays: cfg.relays.clone(),
        watch_pid,
        store_path: store_path(),
        heartbeat: env_duration("TENEX_EDGE_HEARTBEAT_MS", Duration::from_secs(30)),
        obs_interval: env_duration("TENEX_EDGE_OBS_MS", Duration::from_secs(5)),
        status_ttl: status_ttl_duration(),
        turn_first: Duration::from_secs(env_u64("TENEX_EDGE_TURN_FIRST_S", 30)),
        // 0 = disabled: the title re-distills on each new user message, so an
        // in-turn safety re-distill is opt-in (set TENEX_EDGE_TURN_REPEAT_S > 0).
        turn_repeat: Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 0)),
    }
}

pub(in crate::daemon::server) fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
