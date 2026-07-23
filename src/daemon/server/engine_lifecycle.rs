use super::*;

mod params;
mod reconcile_context;
#[cfg(test)]
mod tests;

pub(in crate::daemon::server) use params::engine_params_for;

pub(in crate::daemon::server) async fn spawn_session(
    state: &Arc<DaemonState>,
    params: EngineParams,
) -> Result<()> {
    let pubkey = params.identity.pubkey.clone();
    let runtime_generation = params.runtime_generation;
    let channel = params.channel.clone();

    tracing::info!(
        agent = %params.identity.slug,
        channel = %channel,
        runtime_generation,
        pubkey,
        "spawning session engine"
    );

    let cancel = Arc::new(Notify::new());
    {
        let mut sessions = state.runtime.engines.lock().unwrap();
        if let Some(previous) = sessions.get(&pubkey) {
            if previous.runtime_generation >= runtime_generation {
                anyhow::bail!("pubkey {pubkey} already has an active runtime");
            }
            // The store has already admitted a newer generation. Retain a
            // cancellation permit for the old engine even if it is still in
            // startup I/O and has not reached its select loop yet.
            previous.cancel.notify_one();
        }
        sessions.insert(
            pubkey.clone(),
            SessionHandle {
                cancel: cancel.clone(),
                runtime_generation,
            },
        );
    }
    state.runtime.hosted.lock().unwrap().insert(
        pubkey.clone(),
        HostedAgent {
            keys: params.keys.clone(),
        },
    );
    let st = state.clone();
    let channel_for_sub = channel.clone();
    tokio::spawn(async move {
        if let Err(e) = sync_subscriptions(&st).await {
            tracing::warn!(
                channel = %channel_for_sub,
                error = %e,
                "session subscription setup failed"
            );
        }
    });

    let st = state.clone();
    let task_pubkey = pubkey.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    let status = state.reconcilers.status.clone();
    let presence_publisher = state.reconcilers.presence_publisher.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(
            params,
            provider,
            store,
            cancel,
            status,
            presence_publisher,
        )
        .await;
        if let Err(e) = res {
            tracing::warn!(pubkey = %task_pubkey, runtime_generation, error = %e, "session task exited with error");
        }
        let owns_generation = {
            let mut sessions = st.runtime.engines.lock().unwrap();
            if sessions
                .get(&task_pubkey)
                .is_some_and(|handle| handle.runtime_generation == runtime_generation)
            {
                sessions.remove(&task_pubkey);
                true
            } else {
                false
            }
        };
        if !owns_generation {
            tracing::debug!(pubkey = %task_pubkey, runtime_generation, "ignoring stale runtime teardown");
            return;
        }
        let teardown = match st.with_store(|store| store.get_session(&task_pubkey)) {
            Ok(Some(session))
                if session.runtime_generation == runtime_generation
                    && session.runtime_state == crate::state::RuntimeState::Running =>
            {
                super::managed_lifecycle::stop_generation(
                    &st,
                    &session,
                    crate::state::StopReason::Crash,
                    now_secs(),
                )
                .await
            }
            Ok(_) => Ok(false),
            Err(error) => Err(error),
        };
        match teardown {
            Ok(true) => {}
            Ok(false) => tracing::debug!(
                pubkey = %task_pubkey,
                runtime_generation,
                "engine exit ignored stale runtime generation"
            ),
            Err(e) => tracing::error!(
                pubkey = %task_pubkey,
                runtime_generation,
                error = %e,
                "engine exit: conditional teardown failed"
            ),
        }
        prune_hosted(&st);
        tracing::info!(pubkey = %task_pubkey, runtime_generation, "session engine exited");
    });
    Ok(())
}

pub(in crate::daemon::server) fn prune_hosted(state: &Arc<DaemonState>) {
    let live: std::collections::HashSet<String> = state
        .with_store(|s| s.list_running_sessions().unwrap_or_default())
        .into_iter()
        .map(|r| r.pubkey)
        .collect();
    state
        .runtime
        .hosted
        .lock()
        .unwrap()
        .retain(|pk, _| live.contains(pk));
}

pub(in crate::daemon::server) fn cancel_session(
    state: &Arc<DaemonState>,
    pubkey: &str,
    runtime_generation: u64,
) -> bool {
    if let Some(h) = state
        .runtime
        .engines
        .lock()
        .unwrap()
        .get(pubkey)
        .filter(|handle| handle.runtime_generation == runtime_generation)
    {
        // `notify_one` retains a permit when the engine is still doing startup
        // I/O; `notify_waiters` would lose cancellation before `notified()` is
        // first polled and leave a stopped generation occupying the runtime map.
        h.cancel.notify_one();
        true
    } else {
        false
    }
}

/// Revive sessions a previous daemon left behind (skew re-exec / crash),
/// rebuilding from the `sessions` table. Invariant: **a running session is never
/// reaped and left with an orphaned supervisor.** For each ALIVE row we respawn
/// the engine task iff the session is still live ([`session_still_live`]: PID
/// running AND, for PTY sessions, the supervisor socket accepts a connect+write).
/// A genuinely-gone session is stopped (so `who`/presence don't lie after a
/// restart). The one case where a *running*
/// session is retired — its agent's identity config changed under it — first
/// kills the PTY supervisor, so no orphan is left (defect #4). Transient
/// conditions — a cold relay
/// (`ChannelGate::Degraded`) or a spawn hiccup — no longer reap a running session;
/// correctness on a not-yet-ready channel is upheld by the send-time gate.
pub(in crate::daemon::server) async fn reconcile_sessions(
    state: &Arc<DaemonState>,
    snaps: Vec<crate::state::Session>,
) {
    super::managed_lifecycle::replay_supervisor_exits(state).await;
    super::managed_lifecycle::reconcile_stopping(state).await;
    tracing::info!(
        session_count = snaps.len(),
        "reconciling sessions from previous daemon instance"
    );
    for snap in snaps {
        let pubkey = snap.pubkey.clone();
        let runtime_generation = snap.runtime_generation;
        if state
            .with_store(|store| store.is_mcp_actor_pubkey(&pubkey))
            .unwrap_or(false)
        {
            tracing::debug!(pubkey, "retaining daemon-owned MCP actor session");
            continue;
        }
        // Only a genuinely-gone session is reaped. `child_pid` liveness alone is
        // unsafe (PIDs recycle, reviving a ghost), so for PTY sessions a
        // reachable supervisor socket (connect+write) is the authoritative
        // signal (risk #1).
        if !session_still_live(state, &snap) {
            tracing::warn!(
                pubkey,
                runtime_generation,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                pid = ?snap.child_pid,
                "session process gone (pid stopped or pty socket unreachable); stopping runtime and retaining standing"
            );
            if snap.child_pid.is_some_and(pid_alive)
                && snap.admitted_transport
                    == crate::session_host::transport::TransportKind::Pty.as_str()
            {
                let Some(pty_id) = runtime_endpoint_id(state, &snap, crate::state::LOCATOR_PTY)
                else {
                    tracing::error!(
                        pubkey,
                        runtime_generation,
                        "live PTY supervisor has no endpoint; refusing to lose process ownership"
                    );
                    continue;
                };
                match crate::pty::terminate_owned_supervisor(&pty_id) {
                    Ok(true) => {}
                    Ok(false) => {
                        tracing::error!(pubkey, runtime_generation, pty_id, "unreachable PTY has no owned supervisor metadata; refusing to lose process ownership");
                        continue;
                    }
                    Err(error) => {
                        tracing::error!(pubkey, runtime_generation, pty_id, %error, "unreachable PTY could not be safely terminated; refusing to mark stopped");
                        continue;
                    }
                }
            }
            match super::managed_lifecycle::stop_generation(
                state,
                &snap,
                crate::state::StopReason::Crash,
                now_secs(),
            )
            .await
            {
                Ok(true) => {}
                Ok(false) => tracing::debug!(
                    pubkey,
                    runtime_generation,
                    "reconcile GC ignored stale runtime generation"
                ),
                Err(e) => {
                    tracing::error!(pubkey, runtime_generation, error = %e, "reconcile GC: conditional teardown failed; ghost-running row may remain")
                }
            }
            continue;
        }
        tracing::info!(
            pubkey,
            runtime_generation,
            agent = %snap.agent_slug,
            channel = %snap.channel_h,
            pid = ?snap.child_pid,
            "reviving session from previous daemon instance"
        );
        let identity = match state.with_store(|s| s.session_identity(&pubkey)) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                tracing::warn!(pubkey, "live session disappeared during reconcile");
                continue;
            }
            Err(e) => {
                tracing::warn!(pubkey, error = %e, "session identity reconstruction failed during reconcile; skipping session");
                continue;
            }
        };
        let keys = match state.session_signing_keys(&pubkey) {
            Ok(keys) => keys,
            Err(e) => {
                tracing::warn!(pubkey, error = %e, "session signer reconstruction failed during reconcile; skipping session");
                continue;
            }
        };

        // Re-establish membership + the group-state subscription through the one
        // channel-provisioning primitive. The scope may be a top-level channel
        // (root channel) or a subgroup. Relay-authored parent state wins; the
        // admission-time immediate parent is retained only for a restart before
        // metadata materializes. Idempotent ready channels still fast-path.
        let parent_hint = reconcile_context::parent_hint(state, &snap);
        let (gate, admission_current) = {
            let _lane = state.standing_sync.lock().await;
            let gate = state
                .provider
                .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
                    channel: &snap.channel_h,
                    expect_member: &pubkey,
                    parent_hint: parent_hint.as_deref(),
                    name: None,
                    repair_whitelisted_admins: true,
                })
                .await;
            let admission_current = if matches!(
                gate,
                crate::fabric::nip29::readiness::ChannelGate::Degraded
            ) {
                true
            } else {
                match super::managed_lifecycle::commit_confirmed_admission(
                    state,
                    &pubkey,
                    &snap.channel_h,
                    runtime_generation,
                    snap.lifecycle_epoch,
                )
                .await
                {
                    Ok(current) => current,
                    Err(error) => {
                        tracing::error!(pubkey, %error, "reconciled membership admission could not be persisted");
                        false
                    }
                }
            };
            (gate, admission_current)
        };
        if !admission_current {
            tracing::warn!(
                pubkey,
                runtime_generation,
                "reconciled admission became stale; skipping engine revival"
            );
            continue;
        }
        // `Degraded` means the channel was NOT verified ready on the relay (e.g.
        // the freshly-reconnected relay is still cold / not yet NIP-42 authed).
        // A LIVE session must never be reaped for a transient relay condition, so
        // we revive the engine anyway and log loudly. Correctness is preserved by
        // the send-time readiness gate (per #157), which still refuses to publish
        // into an unverified channel — only session *liveness* is decoupled from
        // relay *readiness* here (risk #2).
        if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
            tracing::warn!(
                pubkey,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                "channel not verified ready on reconcile; reviving live session anyway (send-time gate still enforced), will re-verify during readiness reconciliation"
            );
        }
        if let Err(e) = sync_subscriptions(state).await {
            tracing::warn!(channel = %snap.channel_h, error = %e, "ensure_subscription failed during reconcile");
        }
        let workspace = match reconcile_context::workspace(state, &snap) {
            Ok(workspace) => workspace,
            Err(error) => {
                tracing::error!(
                    pubkey,
                    channel = %snap.channel_h,
                    %error,
                    "cannot reconcile session with unresolved workspace ancestry"
                );
                continue;
            }
        };
        let ep = engine_params_for(
            &state.cfg,
            identity,
            keys,
            runtime_generation,
            &snap.channel_h,
            &workspace,
            "",
            None,
            snap.child_pid,
        );
        if let Err(e) = spawn_session(state, ep).await {
            // The supervisor process is still running (we checked above), so do NOT
            // stop the session — that would blink a running agent offline.
            // Leave the row ALIVE with its signer reserved; the next daemon
            // restart's reconcile retries the engine spawn.
            tracing::error!(
                pubkey,
                runtime_generation,
                error = %e,
                "reconcile: failed to respawn session engine for a running session; leaving row running for retry on next restart"
            );
        }
    }
}

fn runtime_endpoint_id(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    locator_kind: &str,
) -> Option<String> {
    state
        .with_store(|store| {
            store.runtime_locator_for_session(
                &session.pubkey,
                session.runtime_generation,
                locator_kind,
            )
        })
        .ok()?
        .map(|locator| locator.locator_value)
}

// Single source of truth for pid liveness (defect #17). The `pid > 0` guard
// (defect #3/#389) lives with the definition in `crate::liveness`.
pub(in crate::daemon::server) use crate::liveness::pid_alive;

/// Whether reconcile should treat a session left behind by a previous daemon as
/// still running (and therefore revive it rather than reap it).
///
/// `child_pid` liveness alone is unsafe: PIDs recycle, so a dead hosted endpoint's
/// reused PID could revive a ghost session against an unrelated process. A hosted
/// locator delegates the authoritative check to its transport; native harness
/// sessions with no hosted endpoint fall back to the PID check alone (risk #1).
pub(in crate::daemon::server) fn session_still_live(
    state: &Arc<DaemonState>,
    snap: &crate::state::Session,
) -> bool {
    let pid_ok = snap.child_pid.map(pid_alive).unwrap_or(false);
    match state.with_store(|store| crate::session_host::transport::hosted_endpoint_for(store, snap))
    {
        Ok(crate::session_host::transport::HostedEndpoint::Unhosted) => {
            revive_decision(pid_ok, false, None)
        }
        Ok(crate::session_host::transport::HostedEndpoint::Unavailable { kind }) => {
            tracing::warn!(
                pubkey = %snap.pubkey,
                transport = kind.as_str(),
                "admitted hosted endpoint locator is unavailable; refusing PID fallback"
            );
            revive_decision(pid_ok, true, None)
        }
        Ok(crate::session_host::transport::HostedEndpoint::Resolved {
            transport,
            endpoint,
        }) => revive_decision(pid_ok, true, Some(transport.is_live(&endpoint))),
        Err(error) => {
            tracing::error!(
                pubkey = %snap.pubkey,
                admitted_transport = %snap.admitted_transport,
                %error,
                "hosted endpoint lookup failed; refusing PID fallback"
            );
            false
        }
    }
}

/// PID liveness is authoritative only when no hosted transport was admitted.
fn revive_decision(pid_ok: bool, hosted_admitted: bool, endpoint_live: Option<bool>) -> bool {
    match endpoint_live {
        Some(live) => live,
        None if hosted_admitted => false,
        None => pid_ok,
    }
}
