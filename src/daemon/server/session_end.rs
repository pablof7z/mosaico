use super::*;

const CLAIM_GRACE_ENV: &str = "TENEX_EDGE_EPHEMERAL_GRACE_SECS";
const DEFAULT_CLAIM_GRACE_SECS: u64 = 15 * 60;

#[derive(serde::Deserialize)]
pub(in crate::daemon::server) struct SessionEndParams {
    session: String,
}

pub(in crate::daemon::server) async fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionEndParams =
        serde_json::from_value(params.clone()).context("parsing session_end params")?;
    // `get_session` is alias-resolving, so the raw hook id (an alias) yields the
    // canonical session row; every mutation below keys on `rec.session_id`.
    let rec = state.with_store(|s| s.get_session(&p.session).ok().flatten());
    let existed = rec.is_some();
    if let Some(ref rec) = rec {
        let ended_at = now_secs();
        cancel_session(state, &rec.session_id);

        // Release the ordinal reservation + any derived signing key before marking
        // the session dead.
        record_ephemeral_claim(state, rec);
        // Mark the bound identity dead but KEEP the row for route lookup; the
        // explicit claim above controls whether a later mention may resume it.
        state.with_store(|s| s.mark_identity_dead_for_session(&rec.session_id).ok());

        // Mark the canonical session dead (alive=0, working=0). Its final published
        // kind:30315 ages off via NIP-40 expiration.
        state.with_store(|s| {
            s.touch_session(&rec.session_id, ended_at).ok();
            s.mark_dead(&rec.session_id).ok()
        });
        state.outbox_notify.notify_waiters();
        state.emit_tail(TailEvent::Sess {
            ts: ended_at,
            channel: rec.channel_h.clone(),
            agent: rec.agent_slug.clone(),
            session: rec.session_id.clone(),
            state: "end".into(),
            rel_cwd: String::new(),
        });
        // Reconcile subscriptions: the session is now dead, so its scope is closed
        // and any REQ it SOLELY owned is torn down with a real NIP-01 CLOSE. A
        // channel another live session still holds stays open (refcounted).
        super::subscriptions::reconcile_subs_logged(state, "session_end").await;
    }
    Ok(serde_json::json!({ "ended": existed }))
}

#[derive(serde::Deserialize)]
struct SessionKillParams {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    pty_id: Option<String>,
    #[serde(default)]
    revoke_memberships: bool,
}

pub(in crate::daemon::server) async fn rpc_session_kill(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let p: SessionKillParams =
        serde_json::from_value(params.clone()).context("parsing session_kill params")?;
    // Operator callers select by public identity. Self/lifecycle callers own a
    // typed runtime locator, which remains a private fallback until the run
    // spine is separated from session identity.
    let selector = p.session.as_deref().filter(|session| !session.is_empty());
    let public = selector
        .map(|session| state.with_store(|s| super::resolution::resolve_public_session(s, session)))
        .transpose()?
        .flatten();
    let Some(rec) = public.or_else(|| {
        selector.and_then(|session| state.with_store(|s| s.get_session(session).ok().flatten()))
    }) else {
        return kill_unbound_endpoint(p.pty_id.as_deref(), p.revoke_memberships);
    };

    let stop = stop_local_process(state, &rec);
    let ended = rpc_session_end(
        state,
        &serde_json::json!({
            "session": rec.session_id,
        }),
    )
    .await?
    .get("ended")
    .and_then(serde_json::Value::as_bool)
    .unwrap_or(false);

    let cleanup_failures = if p.revoke_memberships {
        revoke_operator_session(state, &rec).await
    } else {
        Vec::new()
    };

    match stop {
        Ok(note) => Ok(serde_json::json!({
            "killed": true,
            "ended": ended,
            "note": note,
            "cleanup_confirmed": cleanup_failures.is_empty(),
            "cleanup_failures": cleanup_failures,
        })),
        Err(e) => Ok(serde_json::json!({
            "killed": false,
            "ended": ended,
            "reason": format!("{e:#}"),
        })),
    }
}

fn kill_unbound_endpoint(
    pty_id: Option<&str>,
    revoke_memberships: bool,
) -> Result<serde_json::Value> {
    let Some(pty_id) = pty_id else {
        return Ok(serde_json::json!({
            "killed": false,
            "ended": false,
            "reason": "no local session matched"
        }));
    };
    let Some(endpoint) = crate::pty::read_all_metadata()
        .into_iter()
        .find(|metadata| metadata.id == pty_id && crate::pty::is_live(&metadata.id))
    else {
        return Ok(serde_json::json!({
            "killed": false,
            "ended": false,
            "reason": "no live local endpoint matched"
        }));
    };
    crate::pty::kill(&endpoint.id)
        .with_context(|| format!("killing unbound PTY endpoint {}", endpoint.id))?;
    let cleanup_failures = if revoke_memberships {
        vec![
            "endpoint had no current session identity; fabric cleanup could not be confirmed"
                .to_string(),
        ]
    } else {
        Vec::new()
    };
    Ok(serde_json::json!({
        "killed": true,
        "ended": false,
        "note": format!("pty={}", endpoint.id),
        "cleanup_confirmed": cleanup_failures.is_empty(),
        "cleanup_failures": cleanup_failures,
    }))
}

async fn revoke_operator_session(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Vec<String> {
    let now = now_secs();
    let mut failures = Vec::new();
    if let Err(error) =
        state.with_store(|store| store.clear_session_claim_for_session(&rec.session_id))
    {
        failures.push(format!("session claim cleanup: {error:#}"));
    }
    match state.session_signing_keys(&rec.agent_pubkey) {
        Ok(keys) => {
            crate::status_seam::drive(
                &state.status,
                state.fabric_provider(),
                &keys,
                &state.store,
                &state.outbox,
                crate::status_seam::DriveMeta {
                    trigger: "operator_session_revoke",
                    window_hash: None,
                    replay_fact: Some(crate::reconcile::InputFact::StatusDrive(
                        crate::reconcile::StatusDrive::SessionRevoked {
                            session_id: rec.session_id.clone(),
                            at: now,
                        },
                    )),
                },
                |status| status.on_session_revoked(&rec.session_id, now),
            )
            .await;
            state.outbox_notify.notify_waiters();
        }
        Err(error) => failures.push(format!("status expiration: {error:#}")),
    }
    failures.extend(
        super::membership_cleanup::revoke_session_memberships(state, &rec.session_id).await,
    );
    failures
}

fn stop_local_process(state: &Arc<DaemonState>, rec: &crate::state::Session) -> Result<String> {
    if let Some(pty_id) = pty_session_for_session(state, &rec.session_id) {
        crate::pty::kill(&pty_id).with_context(|| format!("killing PTY session {pty_id}"))?;
        state.with_store(|s| s.clear_pty_session(&rec.session_id).ok());
        return Ok(format!("pty={pty_id}"));
    }
    if let Some(pid) = rec.child_pid {
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            Some(nix::sys::signal::Signal::SIGTERM),
        )
        .with_context(|| format!("sending SIGTERM to pid {pid}"))?;
        return Ok(format!("pid={pid}"));
    }
    Ok(String::new())
}

fn pty_session_for_session(state: &Arc<DaemonState>, session_id: &str) -> Option<String> {
    state
        .with_store(|s| s.aliases_for_session(session_id))
        .ok()
        .and_then(|aliases| {
            aliases
                .into_iter()
                .find(|a| a.external_id_kind == "pty_session")
                .map(|a| a.external_id)
        })
}

fn record_ephemeral_claim(state: &Arc<DaemonState>, rec: &crate::state::Session) {
    if rec.channel_h.is_empty()
        || !crate::session_host::agent_supports_headless_exec(&rec.agent_slug)
    {
        return;
    }
    let Some(identity) =
        state.with_store(|s| s.identity_for_session(&rec.session_id).ok().flatten())
    else {
        return;
    };
    let native_id = super::pty_rpc::resume_token_for(rec)
        .filter(|s| !s.is_empty())
        .or_else(|| (!identity.native_id.is_empty()).then_some(identity.native_id.clone()));
    let Some(native_id) = native_id else {
        return;
    };

    let now = now_secs();
    let claim = crate::state::session_claims::SessionClaim {
        pubkey: identity.pubkey,
        agent_slug: identity.agent_slug,
        codename: identity.codename,
        session_id: rec.session_id.clone(),
        channel_h: rec.channel_h.clone(),
        native_id,
        harness: rec.harness.clone(),
        last_active_at: now,
        expires_at: now.saturating_add(claim_grace_secs()),
        owner_backend_pubkey: state.backend_pubkey().unwrap_or_default(),
        owner_host: state.host.clone(),
    };
    if let Err(e) = state.with_store(|s| s.upsert_session_claim(&claim)) {
        tracing::warn!(
            session = %rec.session_id,
            agent = %rec.agent_slug,
            channel = %rec.channel_h,
            error = %e,
            "failed to record ephemeral session claim"
        );
    }
}

fn claim_grace_secs() -> u64 {
    std::env::var(CLAIM_GRACE_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_CLAIM_GRACE_SECS)
}
