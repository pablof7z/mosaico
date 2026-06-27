use super::*;

pub(in crate::daemon::server) async fn spawn_session(
    state: &Arc<DaemonState>,
    params: EngineParams,
) -> Result<()> {
    let session_id = params.session_id.clone();
    let pubkey = params.agent_pubkey.clone();
    let project = params.project.clone();

    state.hosted.lock().unwrap().insert(
        pubkey.clone(),
        HostedAgent {
            keys: params.keys.clone(),
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
    state.liveness_changed.notify_waiters();

    let st = state.clone();
    let sid = session_id.clone();
    let proj = project.clone();
    let provider = state.provider.clone();
    let store = state.store.clone();
    tokio::spawn(async move {
        let res = runtime::run_session_in_daemon(params, provider, store, cancel).await;
        if let Err(e) = res {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] session {sid} task error: {e:#}");
            }
        }
        // Engine self-exit path: remove a transient duplicate signer from the
        // NIP-29 group. The Mutex pop is atomic: if rpc_session_end already
        // removed the key, this finds None and avoids a duplicate publish.
        {
            let maybe_key = st.release_session_signer(&sid);
            if let Some(sk) = maybe_key {
                let session_pubkey = sk.public_key().to_hex();
                st.provider
                    .nip29_remove_member(&proj, &session_pubkey)
                    .await;
                st.with_store(|s| {
                    s.remove_group_member(&proj, &session_pubkey).ok();
                });
            }
        }
        // Clear the legacy DB routing row regardless of whether the in-memory key
        // was still present, and mark the (pubkey, h) route dead but keep it for
        // resume (issue #47).
        st.with_store(|s| {
            s.remove_session_pubkeys_for_session(&sid).ok();
            s.mark_identity_route_dead(&sid, now_secs()).ok();
        });
        st.sessions.lock().unwrap().remove(&sid);
        prune_hosted(&st);
        st.liveness_changed.notify_waiters();
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

pub(in crate::daemon::server) async fn ensure_subscription(
    state: &Arc<DaemonState>,
    project: &str,
) -> Result<()> {
    {
        let mut projs = state.subscribed_projects.lock().unwrap();
        if !projs.iter().any(|p| p == project) {
            projs.push(project.to_string());
        }
    }
    // Incremental add: plan only the NARROW deltas for this newly-tracked channel
    // (one `#h` chat/status/long-form REQ + one group-state REQ), NOT a full
    // aggregate rebuild. Mutating an aggregate makes the relay replay every stored
    // event for every tracked entity; a narrow REQ scoped to just this channel
    // avoids that. The deltas are empty when the channel is already covered (by an
    // aggregate seeded at startup or an earlier narrow add), making this idempotent.
    let reqs = state.subscriptions.lock().unwrap().add_channel(project);
    if reqs.is_empty() {
        return Ok(());
    }
    // Bounded: opening a relay subscription can hang on a slow/unreachable relay,
    // and this is awaited on hook-critical paths (session_start, spawn_session),
    // so a hang would block the editor. The intent (project recorded above +
    // folded into the registry) survives a timeout; we fail open.
    match tokio::time::timeout(std::time::Duration::from_secs(5), apply_plan(state, reqs)).await {
        Ok(r) => r,
        Err(_) => {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!("[daemon] subscription apply timed out for {project} (best-effort)");
            }
            Ok(())
        }
    }
}

/// Open each planned REQ under its semantic [`SubscriptionId`], on the MAIN
/// relays only. Broad `#h`/`#p` aggregate filters must NOT hit the kind:0 indexer
/// relay — that relay is a one-shot profile-resolution endpoint, and pinning a
/// firehose there wastes its connection and pulls in noise. Re-applying the same
/// id REPLACES the relay-side REQ in place (NIP-01), which is exactly how `seed`
/// compacts the three aggregates.
pub(in crate::daemon::server) async fn apply_plan(
    state: &Arc<DaemonState>,
    reqs: Vec<crate::fabric::subscriptions::PlannedReq>,
) -> Result<()> {
    for req in reqs {
        state
            .transport
            .subscribe_with_id_to(&state.cfg.relays, req.id, req.filter)
            .await?;
    }
    Ok(())
}

/// Force the relay to replay channel `h`'s stored chat so a session that just
/// became alive receives messages published BEFORE it existed (the spawn-on-
/// mention case: the triggering kind:9 arrives, spawns the agent, but the live
/// materialize path can only route to sessions already alive). Re-applying the
/// channel's narrow `#h` REQ replaces it in place (NIP-01) and the relay
/// re-streams the stored events, which `materialize_chat_message` then routes to
/// the now-alive session. Best-effort: a replay failure just means the session
/// relies on subsequent live chat. Bounded so a slow relay can't block the hook.
pub(in crate::daemon::server) async fn replay_channel_chat(state: &Arc<DaemonState>, h: &str) {
    let req = crate::fabric::subscriptions::channel_chat_replay_req(h);
    let fut = apply_plan(state, vec![req]);
    if tokio::time::timeout(std::time::Duration::from_secs(5), fut)
        .await
        .is_err()
        && std::env::var("TENEX_EDGE_DEBUG").is_ok()
    {
        eprintln!("[daemon] channel chat replay timed out for {h} (best-effort)");
    }
}

/// Close each subscription id (NIP-01 CLOSE). Used when compaction retires the
/// narrow REQs now subsumed by a rebuilt aggregate. Best-effort per id.
#[allow(dead_code)]
pub(in crate::daemon::server) async fn close_subs(
    state: &Arc<DaemonState>,
    ids: Vec<nostr_sdk::prelude::SubscriptionId>,
) -> Result<()> {
    for id in ids {
        state.transport.unsubscribe(&id).await?;
    }
    Ok(())
}

/// Compute the daemon's current subscription coverage from durable sources.
///
/// - `channels_h` / `group_state_d`: explicitly tracked projects, channels live
///   sessions route under, groups any local/ordinal pubkey is a member of, and
///   groups this daemon owns.
/// - `addressed_pubkeys_p`: local durable + ordinal pubkeys, live transient
///   session keys, and the backend identity (folds in the old standalone backend
///   orchestration `#p` subscription).
fn build_entity_coverage(
    state: &Arc<DaemonState>,
) -> crate::fabric::subscriptions::EntityCoverage {
    use std::collections::BTreeSet;

    let edge = crate::config::edge_home();
    let local_pks = crate::identity::list_local_pubkeys(&edge);

    let mut channels: BTreeSet<String> = state
        .subscribed_projects
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect();
    let mut pubkeys: BTreeSet<String> = local_pks.iter().cloned().collect();

    state.with_store(|s| {
        let ordinals = s.list_agent_ordinal_pubkeys();
        pubkeys.extend(ordinals.iter().cloned());
        // Groups any local/ordinal pubkey is a member of (spawn-on-mention path).
        for pk in local_pks.iter().chain(ordinals.iter()) {
            if let Ok(gs) = s.list_groups_for_member(pk) {
                channels.extend(gs);
            }
        }
        // Groups this daemon created.
        if let Ok(owned) = s.list_owned_groups() {
            channels.extend(owned);
        }
        // Channels live sessions currently route under (channel or per-session room).
        for sess in s.list_alive_sessions().unwrap_or_default() {
            channels.insert(sess.route_scope().to_string());
        }
    });

    // Live transient session keys + backend identity round out the addressed set.
    pubkeys.extend(state.live_session_pubkeys());
    if let Some(bp) = state.backend_pubkey() {
        pubkeys.insert(bp.to_string());
    }

    crate::fabric::subscriptions::EntityCoverage {
        channels_h: channels.clone(),
        group_state_d: channels,
        addressed_pubkeys_p: pubkeys,
    }
}

/// Seed the THREE stable aggregate REQs from the daemon's current coverage. This
/// REPLACES the whole aggregate (the compaction point) and applies exactly three
/// REQs: `#h` (chat/status/long-form over all channels), `#p` (chat/long-form
/// addressed to all durable pubkeys), and group-state (39000/39001/39002 over all
/// group ids). It NO LONGER expands per-(project×kind) `Scope`s and NEVER
/// subscribes kind:0 — profile resolution stays on the on-demand `Transport::fetch`
/// + `profile.rs` cache.
///
/// An aggregate filter with an EMPTY coverage set degenerates to an unscoped
/// firehose over its kinds; such a REQ is skipped (never opened) so a daemon with
/// no channels/pubkeys yet does not pull the whole relay. The registry is still
/// seeded so later narrow adds dedup correctly against the (empty) aggregate.
pub(in crate::daemon::server) async fn resubscribe(state: &Arc<DaemonState>) -> Result<()> {
    let coverage = build_entity_coverage(state);
    // seed() returns the three aggregate REQs in the fixed, tested order
    // [`#h`, `#p`, group-state]; pair each with its set's emptiness so we drop
    // any that would be an unscoped firehose.
    let empties = [
        coverage.channels_h.is_empty(),
        coverage.addressed_pubkeys_p.is_empty(),
        coverage.group_state_d.is_empty(),
    ];
    let reqs = state.subscriptions.lock().unwrap().seed(coverage);
    let reqs: Vec<_> = reqs
        .into_iter()
        .zip(empties)
        .filter_map(|(req, empty)| (!empty).then_some(req))
        .collect();
    apply_plan(state, reqs).await
}

/// Revive sessions a previous daemon left behind (skew re-exec / crash),
/// rebuilding from the canonical `session_state` aggregate. For each ACTIVE
/// session: respawn the engine task if its watched pid is still alive, else end
/// the canonical session AND mark the runtime row dead (so `who`/presence don't
/// lie after a restart). `watch_pid` lives in the kept `sessions` runtime table
/// (session_state carries no pid), so it is joined per session.
pub(in crate::daemon::server) async fn reconcile_sessions(state: &Arc<DaemonState>) {
    let now = now_secs();
    let snaps = state.with_store(|s| s.live_session_snapshots(None, 0).unwrap_or_default());
    for snap in snaps {
        let session_id = snap.session_id.as_str().to_owned();
        let watch_pid = state
            .with_store(|s| s.get_session(&session_id).ok().flatten())
            .and_then(|r| r.watch_pid);
        let pid_ok = watch_pid.map(pid_alive).unwrap_or(false);
        if !pid_ok {
            // Read the persisted session pubkey BEFORE deleting its row — it is
            // the authoritative value. Re-deriving from session_aliases is only a
            // fallback for rows written before this column existed; preferring the
            // stored pubkey avoids any chance of removing the wrong key (and thus
            // stranding the real one as a live member) if the recovered anchor
            // ever diverges from what session_start used.
            let stored_pubkey = state.with_store(|s| s.session_pubkey_for_session(&session_id));
            state.with_store(|s| {
                s.end_session(&session_id, now).ok();
                s.mark_session_dead(&session_id).ok();
                // Clear DB routing row for the dead session's transient pubkey.
                s.remove_session_pubkeys_for_session(&session_id).ok();
            });
            // Crash-GC: remove the session pubkey from the NIP-29 group.
            if let Some(nsec) = state.cfg.session_ikm_nsec().cloned() {
                if let Ok(op_keys) = nostr_sdk::prelude::Keys::parse(&nsec) {
                    let session_pubkey = stored_pubkey.unwrap_or_else(|| {
                        // Fallback: re-derive. Anchor recovered from session_aliases:
                        //   claude-code / codex → (harness, native_id)
                        //   opencode → anchor = session_id (resume alias only)
                        //   unknown / no rows → ("unknown", session_id)
                        let (harness_kind, anchor) =
                            state.with_store(|s| s.get_session_derivation_anchor(&session_id));
                        identity::derive_session_keys(
                            op_keys.secret_key(),
                            &snap.project,
                            &snap.agent_slug,
                            &harness_kind,
                            &anchor,
                        )
                        .public_key()
                        .to_hex()
                    });
                    let provider = state.provider.clone();
                    let store = state.store.clone();
                    let project = snap.project.clone();
                    tokio::spawn(async move {
                        provider
                            .nip29_remove_member(&project, &session_pubkey)
                            .await;
                        store
                            .lock()
                            .unwrap()
                            .remove_group_member(&project, &session_pubkey)
                            .ok();
                    });
                }
            }
            continue;
        }
        let id = match identity::load_or_create(&config::edge_home(), &snap.agent_slug, now) {
            Ok(i) => i,
            Err(_) => continue,
        };
        // Re-establish ownership/membership + the group-state subscription for
        // revived sessions, through the one channel-provisioning primitive. The
        // session's scope may be a top-level project (root channel) or a subgroup;
        // its stored parent (if any) is the readiness gate's parent_hint.
        // Idempotent: the owned_groups/group_members cache persists across
        // restarts, so already-ready channels fast-path.
        let parent_hint = state.with_store(|s| s.group_parent(&snap.project).unwrap_or(None));
        state
            .provider
            .ensure_channel_ready(crate::fabric::nip29::readiness::ChannelCtx {
                channel: &snap.project,
                expect_member: &id.pubkey_hex(),
                parent_hint: parent_hint.as_deref(),
            })
            .await;

        let (harness_kind, native_id) =
            state.with_store(|s| s.get_session_derivation_anchor(&session_id));
        let signer = match select_session_signer(
            state,
            &session_id,
            &id.keys,
            &id.pubkey_hex(),
            &snap.agent_slug,
            &snap.project,
            &harness_kind,
            &native_id,
            None,
        ) {
            Ok(signer) => signer,
            Err(e) => {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] signer selection failed while reconciling {}: {e:#}",
                        session_id
                    );
                }
                continue;
            }
        };
        if let Some(member_pubkey) = signer.member_pubkey_to_admit() {
            if let Err(e) = admit_transient_signer(state, &snap.project, member_pubkey).await {
                if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                    eprintln!(
                        "[daemon] ordinal signer admission failed while reconciling {}: {e:#}",
                        session_id
                    );
                }
                state.release_session_signer(&session_id);
                state.with_store(|s| {
                    s.remove_session_pubkeys_for_session(&session_id).ok();
                    s.mark_identity_route_dead(&session_id, now_secs()).ok();
                });
                continue;
            }
        }

        if let Err(e) = ensure_subscription(state, &snap.project).await {
            if std::env::var("TENEX_EDGE_DEBUG").is_ok() {
                eprintln!(
                    "[daemon] ensure_subscription({}) failed: {e:#}",
                    snap.project
                );
            }
        }
        let ep = engine_params_for(
            &state.cfg,
            &id,
            &snap.agent_slug,
            &session_id,
            &snap.project,
            &snap.rel_cwd,
            watch_pid,
            signer.session_keys(),
        );
        let _ = spawn_session(state, ep).await;
    }
    // Any registration/end transitions above enqueued publishes.
    state.status_outbox_notify.notify_waiters();
}

#[allow(clippy::too_many_arguments)]
pub(in crate::daemon::server) fn engine_params_for(
    cfg: &Config,
    id: &AgentIdentity,
    agent_slug: &str,
    session_id: &str,
    project: &str,
    rel_cwd: &str,
    watch_pid: Option<i32>,
    // Derived keypair for a duplicate live session in the same routing scope.
    // `None` keeps the durable agent key as the default signer.
    session_keys: Option<Keys>,
) -> EngineParams {
    EngineParams {
        agent_slug: agent_slug.to_string(),
        agent_pubkey: id.pubkey_hex(),
        keys: id.keys.clone(),
        session_keys,
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
        status_ttl: Duration::from_secs(env_u64("TENEX_EDGE_STATUS_TTL_S", 90)),
        turn_first: Duration::from_secs(env_u64("TENEX_EDGE_TURN_FIRST_S", 30)),
        // 0 = disabled: the title re-distills on each new user message, so an
        // in-turn safety re-distill is opt-in (set TENEX_EDGE_TURN_REPEAT_S > 0).
        turn_repeat: Duration::from_secs(env_u64("TENEX_EDGE_TURN_REPEAT_S", 0)),
    }
}

pub(in crate::daemon::server) fn pid_alive(pid: i32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}
