use super::*;

mod params;
mod session_watch;

pub(in crate::daemon::server) use params::engine_params_for;

pub(in crate::daemon::server) async fn spawn_session(
    state: &Arc<DaemonState>,
    params: EngineParams,
) -> Result<()> {
    let session_id = params.session_id.clone();
    let pubkey = params.identity.pubkey.clone();
    let channel = params.channel.clone();
    let watch_pid = params.watch_pid;

    tracing::info!(
        agent = %params.identity.slug,
        channel = %channel,
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
    let channel_for_sub = channel.clone();
    tokio::spawn(async move {
        if let Err(e) = ensure_subscription(&st, &channel_for_sub).await {
            tracing::warn!(
                channel = %channel_for_sub,
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
        &channel,
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
    let cleaned = state.with_store(|s| s.cleanup_orphan_durable_sessions().unwrap_or_default());
    if cleaned > 0 {
        tracing::warn!(
            cleaned,
            "released orphan durable-agent claims during startup reconcile"
        );
    }
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
                "session process dead; marking dead and leaving membership for stale cleanup"
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
            continue;
        }
        tracing::info!(
            session = %session_id,
            agent = %snap.agent_slug,
            channel = %snap.channel_h,
            pid = ?snap.child_pid,
            "reviving session from previous daemon instance"
        );
        let agent_identity = match crate::identity::load_or_create(
            &crate::config::edge_home(),
            &snap.agent_slug,
            now,
        ) {
            Ok(identity) => identity,
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "agent config load failed during reconcile; skipping session");
                continue;
            }
        };
        if let Err(e) = validate_live_session_identity(state, &snap, &agent_identity) {
            tracing::warn!(session = %session_id, error = %e, "live session identity configuration changed; retiring stale session");
            state.with_store(|s| {
                s.mark_dead(&session_id).ok();
                s.mark_identity_dead_for_session(&session_id).ok();
            });
            continue;
        }
        let minted = match mint_session_identity(
            state,
            &session_id,
            &agent_identity,
            &snap.channel_h,
            &snap.resume_id,
            None,
        ) {
            Ok(minted) => minted,
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "identity mint failed during reconcile; skipping session");
                continue;
            }
        };

        // Re-establish membership + the group-state subscription through the one
        // channel-provisioning primitive. The scope may be a top-level channel
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
            None,
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

pub(in crate::daemon::server) fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
