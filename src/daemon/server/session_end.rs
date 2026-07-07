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
        cancel_session(state, &rec.session_id);

        membership_cleanup::remove_session_memberships(state, &rec.session_id, "session-end");
        // Release the ordinal reservation + any derived signing key before marking
        // the session dead.
        let _session_key = state.release_session_signer(&rec.session_id);
        record_ephemeral_claim(state, rec);
        // Mark the bound identity dead but KEEP the row for route lookup; the
        // explicit claim above controls whether a later mention may resume it.
        state.with_store(|s| s.mark_identity_dead_for_session(&rec.session_id).ok());

        // Mark the canonical session dead (alive=0, working=0). Its final published
        // kind:30315 ages off via NIP-40 expiration.
        state.with_store(|s| s.mark_dead(&rec.session_id).ok());
        state.outbox_notify.notify_waiters();
        state.emit_tail(TailEvent::Sess {
            ts: now_secs(),
            project: rec.channel_h.clone(),
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
        base_pubkey: identity.base_pubkey,
        agent_slug: identity.agent_slug,
        ordinal: identity.ordinal,
        session_id: rec.session_id.clone(),
        channel_h: rec.channel_h.clone(),
        native_id,
        harness: rec.harness.clone(),
        last_active_at: now,
        expires_at: now.saturating_add(claim_grace_secs()),
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
