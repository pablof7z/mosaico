use super::*;

pub(in crate::daemon::server) async fn spawn_session(
    state: &Arc<DaemonState>,
    params: EngineParams,
) -> Result<()> {
    let session_id = params.session_id.clone();
    let pubkey = params.instance.pubkey.clone();
    let project = params.project.clone();

    tracing::info!(
        agent = %params.instance.base_slug,
        channel = %project,
        session = %session_id,
        "spawning session engine"
    );

    // Register THIS instance's signing keys under its selected pubkey, so
    // `keys_for(selected_pubkey)` returns the key that actually authored its
    // events (base key for ordinal 0, derived key for ordinal N).
    state.hosted.lock().unwrap().insert(
        pubkey.clone(),
        HostedAgent {
            keys: params.instance.signing_keys(&params.base_keys),
        },
    );
    ensure_subscription(state, &project).await?;

    let cancel = Arc::new(Notify::new());
    state.sessions.lock().unwrap().insert(
        session_id.clone(),
        SessionHandle {
            cancel: cancel.clone(),
        },
    );

    let st = state.clone();
    let sid = session_id.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(params, provider, store, cancel).await;
        if let Err(e) = res {
            tracing::warn!(session = %sid, error = %e, "session task exited with error");
        }
        membership_cleanup::remove_session_memberships(&st, &sid, "engine-exit");
        st.release_session_signer(&sid);
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
        let id = match identity::load_or_create(&config::edge_home(), &snap.agent_slug, now) {
            Ok(i) => i,
            Err(_) => continue,
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
                expect_member: &id.pubkey_hex(),
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
            continue;
        }

        let signer = match select_session_signer(
            state,
            &session_id,
            &id.keys,
            &id.pubkey_hex(),
            &snap.agent_slug,
            &snap.channel_h,
            &snap.resume_id,
            None,
        ) {
            Ok(signer) => signer,
            Err(e) => {
                tracing::warn!(session = %session_id, error = %e, "signer selection failed during reconcile; skipping session");
                continue;
            }
        };
        // Rebind the row to the selected ordinal pubkey (== base for ordinal 0) so
        // mention routing keys on this session's real identity, not the base.
        state.with_store(|s| {
            if let Err(e) = s.set_session_agent_pubkey(&session_id, &signer.pubkey) {
                tracing::error!(
                    session = %session_id,
                    pubkey = %signer.pubkey,
                    error = %e,
                    "reconcile: failed to rebind session to ordinal pubkey; mention routing may key on the base identity"
                );
            }
        });
        if let Some(member_pubkey) = signer.member_pubkey_to_admit() {
            if let Err(e) = admit_ordinal_signer(state, &snap.channel_h, member_pubkey).await {
                tracing::warn!(session = %session_id, error = %e, "ordinal signer admission failed during reconcile; skipping session");
                state.release_session_signer(&session_id);
                state.with_store(|s| {
                    if let Err(e) = s.mark_identity_dead_for_session(&session_id) {
                        tracing::error!(session = %session_id, error = %e, "reconcile: failed to mark identity dead after admission failure; ghost ordinal may remain");
                    }
                });
                continue;
            }
        }

        if let Err(e) = ensure_subscription(state, &snap.channel_h).await {
            tracing::warn!(channel = %snap.channel_h, error = %e, "ensure_subscription failed during reconcile");
        }
        let ep = engine_params_for(
            &state.cfg,
            &id,
            signer.instance(&snap.agent_slug, &id.pubkey_hex()),
            &session_id,
            &snap.channel_h,
            "",
            snap.child_pid,
        );
        let _ = spawn_session(state, ep).await;
    }
    // Any registration/end transitions above enqueued publishes.
    state.outbox_notify.notify_waiters();
}

#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn engine_params_for(
    cfg: &Config,
    id: &AgentIdentity,
    // The session's ONE authoritative agent-instance identity (issue #98): base
    // slug/pubkey, selected ordinal + pubkey, and (via its methods) the display
    // label + signing key. The engine derives all wire identity from it.
    instance: crate::identity::AgentInstance,
    session_id: &str,
    project: &str,
    rel_cwd: &str,
    watch_pid: Option<i32>,
) -> EngineParams {
    EngineParams {
        instance,
        // Derivation root for this instance's signing keys (ordinal 0 == this).
        base_keys: id.keys.clone(),
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
