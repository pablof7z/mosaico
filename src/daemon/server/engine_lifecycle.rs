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
        // Only a genuinely-gone session is reaped. `child_pid` liveness alone is
        // unsafe (PIDs recycle, reviving a ghost), so for PTY sessions a
        // reachable supervisor socket (connect+write) is the authoritative
        // signal (risk #1).
        if !session_still_live(state, &snap) {
            tracing::warn!(
                session = %session_id,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                pid = ?snap.child_pid,
                "session process gone (pid dead or pty socket unreachable); marking dead and leaving membership for stale cleanup"
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
            retire_live_session_for_config_change(state, &session_id, snap.child_pid, now);
            continue;
        }
        let minted = match mint_session_identity(
            state,
            &session_id,
            &agent_identity,
            &snap.channel_h,
            SessionIdentityInput::new(&snap.resume_id, None),
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
        // `Degraded` means the channel was NOT verified ready on the relay (e.g.
        // the freshly-reconnected relay is still cold / not yet NIP-42 authed).
        // A LIVE session must never be reaped for a transient relay condition, so
        // we revive the engine anyway and log loudly. Correctness is preserved by
        // the send-time readiness gate (per #157), which still refuses to publish
        // into an unverified channel — only session *liveness* is decoupled from
        // relay *readiness* here (risk #2).
        if matches!(gate, crate::fabric::nip29::readiness::ChannelGate::Degraded) {
            tracing::warn!(
                session = %session_id,
                agent = %snap.agent_slug,
                channel = %snap.channel_h,
                "channel not verified ready on reconcile; reviving live session anyway (send-time gate still enforced), will re-verify on next heartbeat"
            );
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
        let workspace = state.with_store(|s| resolution::work_root_for(s, &snap.channel_h));
        let ep = engine_params_for(
            &state.cfg,
            minted.identity.clone(),
            minted.keys.clone(),
            &session_id,
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
                session = %session_id,
                error = %e,
                "reconcile: failed to respawn session engine for a live session; leaving row alive for retry on next restart"
            );
        }
    }
    // Any registration/end transitions above enqueued publishes.
    state.outbox_notify.notify_waiters();
}

/// Retire a session whose agent identity configuration changed under it while
/// still running. The session is genuinely LIVE (it just passed
/// [`session_still_live`]), so we MUST kill its PTY supervisor **before** marking
/// the row dead — otherwise we leak exactly the orphaned supervisor #382 exists
/// to reap (defect #4). Exec/native sessions with no PTY endpoint metadata have
/// no supervisor to kill; the row is retired directly.
fn retire_live_session_for_config_change(
    state: &Arc<DaemonState>,
    session_id: &str,
    child_pid: Option<i32>,
    now: u64,
) {
    if let Some(sock) = pty_socket_for(state, session_id) {
        if let Err(e) = crate::pty::kill(&sock) {
            tracing::warn!(
                session = %session_id,
                socket = %sock,
                error = %e,
                "failed to kill PTY supervisor while retiring session for identity-config change; supervisor may be orphaned"
            );
        }
    }
    state.with_store(|s| {
        s.mark_dead(session_id).ok();
        s.mark_identity_dead_for_session(session_id).ok();
    });
    session_watch::exited_at(
        state,
        session_id,
        child_pid,
        now,
        "reconcile-identity-config-change",
    );
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
        TransportKind::Acp
    ) {
        return endpoint_id_for(state, &snap.session_id)
            .map(|endpoint_id| {
                AcpTransport.is_live(&EndpointRef {
                    kind: TransportKind::Acp,
                    endpoint_id,
                })
            })
            .unwrap_or(false);
    }
    let pid_ok = snap.child_pid.map(pid_alive).unwrap_or(false);
    let pty_live = pty_socket_for(state, &snap.session_id).map(|sock| crate::pty::is_live(&sock));
    revive_decision(pid_ok, pty_live)
}

/// The ACP endpoint id bound to a session (the `pty_session` alias — for ACP this
/// is the transport endpoint id, not a PTY pane). Used to consult the transport
/// child registry for liveness.
fn endpoint_id_for(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    state.with_store(|s| {
        s.aliases_for_session(session_id).ok().and_then(|aliases| {
            aliases
                .into_iter()
                .find(|a| a.external_id_kind == "pty_session")
                .map(|a| a.external_id)
        })
    })
}

/// Pure revive decision, split out for unit testing. `pty_live` is `None` for a
/// session with no PTY supervisor socket (an exec/native session), in which case
/// the PID check is authoritative.
fn revive_decision(pid_ok: bool, pty_live: Option<bool>) -> bool {
    pid_ok && pty_live.unwrap_or(true)
}

/// The PTY supervisor socket path bound to a session, if it launched under a PTY.
/// The session keeps only the endpoint id alias; PTY launch metadata owns the
/// socket path. An absolute endpoint is accepted for tests and direct callers.
fn pty_socket_for(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    let endpoint_id = endpoint_id_for(state, session_id)?;
    crate::pty::endpoint_socket(&endpoint_id)
}
