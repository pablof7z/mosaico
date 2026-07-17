use super::*;

#[cfg(test)]
#[path = "session_end/tests.rs"]
mod tests;

const CLAIM_GRACE_ENV: &str = "MOSAICO_EPHEMERAL_GRACE_SECS";
const DEFAULT_CLAIM_GRACE_SECS: u64 = 15 * 60;

pub(in crate::daemon::server) async fn rpc_session_end(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let anchor = CallerAnchor::from_params(params);
    let rec = resolve_session_inner(state, &anchor, ResolveScope::Strict).ok();
    let Some(rec) = rec else {
        return Ok(serde_json::json!({ "ended": false }));
    };
    let ended = end_runtime_generation(state, &rec.pubkey, rec.runtime_generation).await?;
    Ok(serde_json::json!({ "ended": ended }))
}

/// End exactly one runtime incarnation. A callback from an older incarnation
/// cannot retire a newer process that reused the same authoritative pubkey.
pub(in crate::daemon::server) async fn end_runtime_generation(
    state: &Arc<DaemonState>,
    pubkey: &str,
    runtime_generation: u64,
) -> Result<bool> {
    let Some(rec) = state.with_store(|store| store.get_session(pubkey))? else {
        return Ok(false);
    };
    if rec.runtime_generation != runtime_generation || !rec.alive {
        return Ok(false);
    }

    let ended_at = now_secs();
    cancel_session(state, pubkey);
    record_resumable_claim(state, &rec);
    let ended = state.with_store(|store| {
        store.touch_session(pubkey, ended_at)?;
        store.mark_dead_if_generation(pubkey, runtime_generation)
    })?;
    if !ended {
        return Ok(false);
    }

    state.emit_tail(TailEvent::Sess {
        ts: ended_at,
        channel: rec.channel_h.clone(),
        agent: rec.agent_slug.clone(),
        session: pubkey.to_string(),
        state: "end".into(),
        rel_cwd: String::new(),
    });
    let state = state.clone();
    tokio::spawn(async move {
        super::subscriptions::reconcile_subs_logged(&state, "session_end").await;
    });
    Ok(true)
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
    let selector = p.session.as_deref().filter(|session| !session.is_empty());
    let public = selector
        .map(|session| state.with_store(|s| super::resolution::resolve_public_session(s, session)))
        .transpose()?
        .flatten();
    let Some(rec) = public else {
        return kill_unbound_endpoint(p.pty_id.as_deref(), p.revoke_memberships);
    };

    let stop = stop_local_process(state, &rec).await;
    let ended = end_runtime_generation(state, &rec.pubkey, rec.runtime_generation).await?;
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
        Err(error) => Ok(serde_json::json!({
            "killed": false,
            "ended": ended,
            "reason": format!("{error:#}"),
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
    if let Err(error) = state.with_store(|store| store.clear_session_claim_for_pubkey(&rec.pubkey))
    {
        failures.push(format!("session claim cleanup: {error:#}"));
    }
    match state.session_signing_keys(&rec.pubkey) {
        Ok(keys) => {
            crate::status_seam::drive(
                &state.reconcilers.status,
                state.fabric_provider(),
                &keys,
                &state.store,
                crate::status_seam::DriveMeta {
                    trigger: "operator_session_revoke",
                },
                |status| status.on_session_revoked(&rec.pubkey, now),
            )
            .await;
        }
        Err(error) => failures.push(format!("status expiration: {error:#}")),
    }
    failures
        .extend(super::membership_cleanup::revoke_session_memberships(state, &rec.pubkey).await);
    failures
}

pub(in crate::daemon::server) async fn stop_local_process(
    state: &Arc<DaemonState>,
    rec: &crate::state::Session,
) -> Result<String> {
    match state
        .with_store(|store| crate::session_host::transport::hosted_endpoint_for(store, rec))?
    {
        crate::session_host::transport::HostedEndpoint::Resolved {
            transport,
            endpoint,
        } => {
            transport
                .kill(&endpoint)
                .await
                .with_context(|| format!("killing {} endpoint", endpoint.kind.as_str()))?;
            state.with_store(|store| {
                store.clear_session_locator_kind(
                    &rec.pubkey,
                    &rec.observed_harness,
                    endpoint.kind.locator_kind(),
                )
            })?;
            return Ok(format!("endpoint={}", endpoint.endpoint_id));
        }
        crate::session_host::transport::HostedEndpoint::Unavailable { kind } => {
            anyhow::bail!(
                "session {} was admitted on {} but its endpoint locator is unavailable; refusing PID fallback",
                rec.pubkey,
                kind.as_str()
            );
        }
        crate::session_host::transport::HostedEndpoint::Unhosted => {}
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

fn record_resumable_claim(state: &Arc<DaemonState>, rec: &crate::state::Session) {
    if rec.channel_h.is_empty()
        || state
            .with_store(|store| store.native_resume_locator(&rec.pubkey, &rec.observed_harness))
            .ok()
            .flatten()
            .is_none()
    {
        return;
    }
    let now = now_secs();
    let claim = crate::state::session_claims::SessionClaim {
        pubkey: rec.pubkey.clone(),
        agent_slug: rec.agent_slug.clone(),
        channel_h: rec.channel_h.clone(),
        harness: rec.observed_harness.clone(),
        last_active_at: now,
        expires_at: now.saturating_add(claim_grace_secs()),
        owner_backend_pubkey: state.backend_pubkey().unwrap_or_default(),
        owner_host: state.host.clone(),
    };
    if let Err(error) = state.with_store(|store| store.upsert_session_claim(&claim)) {
        tracing::warn!(pubkey = %rec.pubkey, error = %error, "failed to record resumable claim");
    }
}

fn claim_grace_secs() -> u64 {
    std::env::var(CLAIM_GRACE_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_CLAIM_GRACE_SECS)
}
