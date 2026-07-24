use super::*;

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionEndCause {
    Manual,
    HarnessHook,
}

#[derive(serde::Deserialize)]
struct SessionEndParams {
    cause: SessionEndCause,
}

pub(in crate::daemon::server) async fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let parsed: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    let anchor = CallerAnchor::from_params(params);
    let Some(session) = resolve_session_inner(state, &anchor, ResolveScope::Strict).ok() else {
        return Ok(serde_json::json!({"ended": false}));
    };
    if matches!(parsed.cause, SessionEndCause::HarnessHook)
        && endpoint_locator(state, &session)
            .is_some_and(|locator| locator.locator_kind == crate::state::LOCATOR_PTY)
    {
        // The PTY supervisor owns the only atomic observation of child status
        // plus attachment state. A harness hook may arrive just before that
        // observation and must not pre-classify a headed clean exit as headless.
        return Ok(serde_json::json!({"ended": false, "deferred": true}));
    }
    let ended = end_runtime_generation(
        state,
        &session.pubkey,
        session.runtime_generation,
        crate::state::StopReason::HeadlessExit,
    )
    .await?;
    Ok(serde_json::json!({"ended": ended}))
}

/// End exactly one runtime incarnation. The explicit reason prevents endpoint
/// exit, operator stop, and idle eviction from collapsing into one lifecycle.
pub(in crate::daemon::server) async fn end_runtime_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    runtime_generation: u64,
    reason: crate::state::StopReason,
) -> Result<bool> {
    let Some(session) = state.with_store(|store| store.get_session(pubkey))? else {
        return Ok(false);
    };
    if session.runtime_generation != runtime_generation || !session.is_running() {
        return Ok(false);
    }
    let ended =
        super::managed_lifecycle::stop_generation(state, &session, reason, now_secs()).await?;
    if ended {
        super::subscriptions::reconcile_subs_logged(state, "session_end").await;
    }
    Ok(ended)
}

#[derive(serde::Deserialize)]
struct SessionKillParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    pty_id: Option<String>,
    #[serde(default)]
    forget: bool,
}

pub(in crate::daemon::server) async fn rpc_session_kill(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: SessionKillParams =
        serde_json::from_value(params.clone()).context("parsing session_kill params")?;
    let selector = params
        .session
        .as_deref()
        .filter(|session| !session.is_empty());
    let public = selector
        .map(|session| {
            state.with_store(|store| super::resolution::resolve_public_session(store, session))
        })
        .transpose()?
        .flatten();
    let Some(session) = public else {
        return kill_unbound_endpoint(params.pty_id.as_deref(), params.forget).await;
    };
    if params.forget {
        return forget_session(state, &session).await;
    }

    let stop = match super::session_termination::terminate_explicit(state, &session).await {
        Ok(note) => note,
        Err(error) => {
            return Ok(serde_json::json!({
                "killed": false,
                "ended": false,
                "reason": format!("{error:#}"),
            }));
        }
    };
    let transitioned = super::managed_lifecycle::stop_generation(
        state,
        &session,
        crate::state::StopReason::OperatorKill,
        now_secs(),
    )
    .await?;
    let ended = transitioned
        || state
            .with_store(|store| store.get_session(&session.pubkey))?
            .is_some_and(|current| {
                current.runtime_generation == session.runtime_generation && !current.is_running()
            });

    Ok(serde_json::json!({
        "killed": true,
        "ended": ended,
        "note": stop,
        "cleanup_confirmed": true,
        "cleanup_failures": [],
    }))
}

async fn forget_session(
    state: &Arc<DaemonState>,
    selected: &crate::state::Session,
) -> Result<serde_json::Value> {
    // Recovery revocation is the first durable write. The standing lane makes
    // the channel/signing snapshots complete with respect to an admission that
    // was already in flight; future reservations fail on recovery_state.
    let (current, channels, signing_keys) = {
        let _lane = state.standing_sync.lock().await;
        let current = revoke_current_generation(state, &selected.pubkey)?;
        super::engine_lifecycle::cancel_session(state, &current.pubkey, current.runtime_generation);
        let channels = super::membership_cleanup::recorded_channels(state, &current.pubkey);
        let signing_keys = state.session_signing_keys(&current.pubkey);
        (current, channels, signing_keys)
    };

    let stop = match super::session_termination::terminate_explicit(state, &current).await {
        Ok(note) => note,
        Err(error) => {
            return Ok(serde_json::json!({
                "killed": false,
                "ended": false,
                "recovery_revoked": true,
                "reason": format!(
                    "recovery was revoked, but runtime termination was not confirmed: {error:#}"
                ),
            }));
        }
    };

    let finalized = {
        let _lane = state.standing_sync.lock().await;
        state.with_store(|store| {
            store.finalize_session_recovery_revocation(
                &current.pubkey,
                current.runtime_generation,
                now_secs(),
            )
        })?
    };
    if !finalized {
        return Ok(serde_json::json!({
            "killed": true,
            "ended": false,
            "recovery_revoked": true,
            "note": stop,
            "reason": "runtime terminated, but recovery finalization lost its generation fence",
        }));
    }

    let cleanup_failures = revoke_operator_session(state, &current, signing_keys, channels).await;
    Ok(serde_json::json!({
        "killed": true,
        "ended": true,
        "recovery_revoked": true,
        "note": stop,
        "cleanup_confirmed": cleanup_failures.is_empty(),
        "cleanup_failures": cleanup_failures,
    }))
}

fn revoke_current_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
) -> Result<crate::state::Session> {
    loop {
        let current = state
            .with_store(|store| store.get_session(pubkey))?
            .with_context(|| format!("session {pubkey} disappeared during recovery revocation"))?;
        if !state.with_store(|store| {
            store.revoke_session_recovery_if_generation(pubkey, current.runtime_generation)
        })? {
            continue;
        }
        let fenced = state
            .with_store(|store| store.get_session(pubkey))?
            .with_context(|| format!("session {pubkey} disappeared after recovery revocation"))?;
        if fenced.runtime_generation == current.runtime_generation
            && fenced.recovery_state == crate::state::RecoveryState::Revoked
        {
            return Ok(fenced);
        }
    }
}

async fn kill_unbound_endpoint(pty_id: Option<&str>, forget: bool) -> Result<serde_json::Value> {
    let Some(pty_id) = pty_id else {
        return Ok(serde_json::json!({
            "killed": false, "ended": false, "reason": "no local session matched"
        }));
    };
    let Some(note) = super::session_termination::terminate_explicit_unbound_pty(pty_id).await?
    else {
        return Ok(serde_json::json!({
            "killed": false, "ended": false, "reason": "no live local endpoint matched"
        }));
    };
    let cleanup_failures = if forget {
        vec!["endpoint has no session identity; recovery cannot be forgotten".to_string()]
    } else {
        Vec::new()
    };
    Ok(serde_json::json!({
        "killed": true,
        "ended": false,
        "note": note,
        "cleanup_confirmed": cleanup_failures.is_empty(),
        "cleanup_failures": cleanup_failures,
    }))
}

async fn revoke_operator_session(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    signing_keys: Result<nostr::Keys>,
    channels: Vec<String>,
) -> Vec<String> {
    let now = now_secs();
    let mut failures = Vec::new();
    match signing_keys {
        Ok(keys) => {
            crate::presence_publisher::drive(
                &state.reconcilers.status,
                &state.reconcilers.presence_publisher,
                &keys,
                crate::presence_publisher::DriveMeta {
                    trigger: "operator_session_revoke",
                },
                |status| status.revoke(&session.pubkey, session.runtime_generation, now),
            );
        }
        Err(error) => failures.push(format!("status expiration: {error:#}")),
    }
    failures.extend(
        super::membership_cleanup::remove_revoked_session_memberships(
            state,
            &session.pubkey,
            channels,
        )
        .await,
    );
    failures
}

fn endpoint_locator(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
) -> Option<crate::state::SessionLocator> {
    state
        .with_store(|store| {
            store
                .runtime_locator_for_session(
                    &session.pubkey,
                    session.runtime_generation,
                    crate::state::LOCATOR_PTY,
                )
                .and_then(|pty| match pty {
                    Some(pty) => Ok(Some(pty)),
                    None => store.runtime_locator_for_session(
                        &session.pubkey,
                        session.runtime_generation,
                        crate::state::LOCATOR_ACP,
                    ),
                })
        })
        .ok()?
}
