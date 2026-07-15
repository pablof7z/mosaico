use super::*;

mod params;
mod session_watch;
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
    let watch_pid = params.watch_pid;

    tracing::info!(
        agent = %params.identity.slug,
        channel = %channel,
        runtime_generation,
        pubkey,
        "spawning session engine"
    );

    let cancel = Arc::new(Notify::new());
    {
        let mut sessions = state.sessions.lock().unwrap();
        if sessions.contains_key(&pubkey) {
            anyhow::bail!("pubkey {pubkey} already has an active runtime");
        }
        sessions.insert(
            pubkey.clone(),
            SessionHandle {
                cancel: cancel.clone(),
                runtime_generation,
            },
        );
    }
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

    session_watch::started(state, &pubkey, &channel, watch_pid, "spawn-session");

    let st = state.clone();
    let task_pubkey = pubkey.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    let status = state.status.clone();
    let outbox = state.outbox.clone();
    tokio::spawn(async move {
        let res =
            runtime::run_session_in_daemon(params, provider, store, cancel, status, outbox).await;
        if let Err(e) = res {
            tracing::warn!(pubkey = %task_pubkey, runtime_generation, error = %e, "session task exited with error");
        }
        let owns_generation = {
            let mut sessions = st.sessions.lock().unwrap();
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
        session_watch::exited(&st, &task_pubkey, watch_pid, "engine-exit");
        match st.with_store(|s| s.mark_dead_if_generation(&task_pubkey, runtime_generation)) {
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
        .with_store(|s| s.list_alive_sessions().unwrap_or_default())
        .into_iter()
        .map(|r| r.pubkey)
        .collect();
    state
        .hosted
        .lock()
        .unwrap()
        .retain(|pk, _| live.contains(pk));
}

pub(in crate::daemon::server) fn cancel_session(state: &Arc<DaemonState>, pubkey: &str) -> bool {
    if let Some(h) = state.sessions.lock().unwrap().get(pubkey) {
        h.cancel.notify_waiters();
        true
    } else {
        false
    }
}

/// Revive sessions a previous daemon left behind (skew re-exec / crash),
/// rebuilding from the `sessions` table. Invariant: **a live session is never
/// reaped and left with an orphaned supervisor.** For each ALIVE row we respawn
/// the engine task iff the session is still live ([`session_still_live`]: PID
/// alive AND, for PTY sessions, the supervisor socket accepts a connect+write).
/// A genuinely-gone session is marked dead (so `who`/presence don't lie after a
/// restart) and has its ordinal member crash-GC'd. The one case where a *live*
/// session is retired — its agent's identity config changed under it — first
/// kills the PTY supervisor, so no orphan is left (defect #4). Transient
/// conditions — a cold relay
/// (`ChannelGate::Degraded`) or a spawn hiccup — no longer reap a live session;
/// correctness on a not-yet-ready channel is upheld by the send-time gate.
pub(in crate::daemon::server) async fn reconcile_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let snaps = state.with_store(|s| s.list_alive_sessions().unwrap_or_default());
    tracing::info!(
        session_count = snaps.len(),
        "reconciling sessions from previous daemon instance"
    );
    for snap in snaps {
        let pubkey = snap.pubkey.clone();
        let runtime_generation = snap.runtime_generation;
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
                "session process gone (pid dead or pty socket unreachable); marking dead and leaving membership for stale cleanup"
            );
            state.with_store(|s| {
                match s.mark_dead_if_generation(&pubkey, runtime_generation) {
                    Ok(true) => {}
                    Ok(false) => tracing::debug!(pubkey, runtime_generation, "reconcile GC ignored stale runtime generation"),
                    Err(e) => {
                        tracing::error!(pubkey, runtime_generation, error = %e, "reconcile GC: conditional teardown failed; ghost-alive row may remain");
                    }
                }
            });
            session_watch::exited_at(state, &pubkey, snap.child_pid, now, "reconcile-dead-pid");
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
                expect_member: &pubkey,
                parent_hint: parent_hint.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            })
            .await;
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
                "channel not verified ready on reconcile; reviving live session anyway (send-time gate still enforced), will re-verify on next heartbeat"
            );
        }
        if let Err(e) = ensure_subscription(state, &snap.channel_h).await {
            tracing::warn!(channel = %snap.channel_h, error = %e, "ensure_subscription failed during reconcile");
        }
        let workspace = state.with_store(|s| resolution::work_root_for(s, &snap.channel_h));
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
            // The supervisor process is still alive (we checked above), so do NOT
            // mark the session dead — that would blink a running agent offline.
            // Leave the row ALIVE with its signer reserved; the next daemon
            // restart's reconcile retries the engine spawn.
            tracing::error!(
                pubkey,
                runtime_generation,
                error = %e,
                "reconcile: failed to respawn session engine for a live session; leaving row alive for retry on next restart"
            );
        }
    }
    // Any registration/end transitions above enqueued publishes.
    state.outbox_notify.notify_waiters();
}

// Single source of truth for pid liveness (defect #17). The `pid > 0` guard
// (defect #3/#389) lives with the definition in `crate::liveness`.
pub(in crate::daemon::server) use crate::liveness::pid_alive;

/// Whether reconcile should treat a session left behind by a previous daemon as
/// still alive (and therefore revive it rather than reap it).
///
/// `child_pid` liveness alone is unsafe: PIDs recycle, so a dead supervisor's
/// reused PID could revive a ghost session against an unrelated process. When the
/// session owns a PTY supervisor socket, that socket being reachable —
/// connect+write, not a round-trip reply ([`crate::pty::is_live`]) — is the
/// authoritative, unspoofable signal; exec sessions with no socket fall back to
/// the PID check alone (risk #1).
fn session_still_live(state: &Arc<DaemonState>, snap: &crate::state::Session) -> bool {
    use crate::session_host::transport::{
        transport_kind_for_slug, AcpTransport, EndpointRef, SessionTransport, TransportKind,
    };
    // ACP/RPC sessions have neither an OS-inspectable supervisor pid nor a PTY
    // socket; their liveness is the in-process child registry (defect #3). That
    // registry cannot survive a daemon restart, so at reconcile it is empty and an
    // ACP session is correctly treated as gone. NEVER fall back to `pid_alive` for
    // ACP: the recorded pid is a synth `0` (or a since-recycled child pid), which
    // would revive an immortal ghost.
    if matches!(
        transport_kind_for_slug(&snap.agent_slug),
        Ok(TransportKind::Acp)
    ) {
        return endpoint_id_for(state, &snap.pubkey, crate::state::LOCATOR_ACP)
            .map(|endpoint_id| {
                AcpTransport.is_live(&EndpointRef {
                    kind: TransportKind::Acp,
                    endpoint_id,
                })
            })
            .unwrap_or(false);
    }
    let pid_ok = snap.child_pid.map(pid_alive).unwrap_or(false);
    let endpoint_live = endpoint_id_for(state, &snap.pubkey, crate::state::LOCATOR_PTY)
        .map(|endpoint_id| crate::pty::is_live(&endpoint_id));
    revive_decision(pid_ok, endpoint_live)
}

/// The typed host endpoint bound to this pubkey, if one exists.
fn endpoint_id_for(state: &Arc<DaemonState>, pubkey: &str, locator_kind: &str) -> Option<String> {
    state.with_store(|s| {
        s.locators_for_pubkey(pubkey).ok().and_then(|locators| {
            locators
                .into_iter()
                .find(|locator| locator.locator_kind == locator_kind)
                .map(|locator| locator.locator_value)
        })
    })
}

/// Pure revive decision, split out for unit testing. `endpoint_live` is `None` for a
/// session with no PTY supervisor socket (an exec/native session), in which case
/// the PID check is authoritative.
fn revive_decision(pid_ok: bool, endpoint_live: Option<bool>) -> bool {
    pid_ok && endpoint_live.unwrap_or(true)
}
