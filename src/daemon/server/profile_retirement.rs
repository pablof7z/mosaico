//! Durable retirement of a reclaimed handle's previous owner.
//!
//! A reclaimed handle's previous owner keeps advertising the handle in its
//! kind:0 until a retirement profile reaches the relay. Session start records
//! that work durably and attempts it in the background, so the hook path
//! stays fail-open on a wedged relay; the lifecycle loop retries due work
//! until the relay acknowledges it, across reconnects and restarts.

use super::*;

/// Record the retirement and attempt it off the hook path.
pub(in crate::daemon::server) fn retire_reclaimed_profile(
    state: &Arc<DaemonState>,
    reclaimed_pubkey: Option<&str>,
) -> Result<()> {
    let Some(pubkey) = reclaimed_pubkey else {
        return Ok(());
    };
    if state
        .with_store(|store| store.get_session(pubkey))?
        .is_none()
    {
        tracing::warn!(pubkey, "reclaimed handle had no local runtime projection");
        return Ok(());
    }
    state.with_store(|store| store.schedule_profile_retirement(pubkey, now_secs()))?;
    let state = state.clone();
    let pubkey = pubkey.to_string();
    tokio::spawn(async move {
        attempt(&state, &pubkey).await;
    });
    Ok(())
}

/// Retry every retirement the relay has not yet acknowledged.
pub(in crate::daemon::server) async fn retry_due(state: &Arc<DaemonState>) {
    let due = state
        .with_store(|store| store.list_due_profile_retirements())
        .unwrap_or_else(|error| {
            tracing::error!(%error, "profile retirement scan failed");
            Vec::new()
        });
    for pubkey in due {
        attempt(state, &pubkey).await;
    }
}

async fn attempt(state: &Arc<DaemonState>, pubkey: &str) {
    let session = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(session)) => session,
        Ok(None) => {
            // Nothing local signs for this pubkey any more.
            complete(state, pubkey);
            return;
        }
        Err(error) => {
            tracing::warn!(pubkey, %error, "profile retirement read failed");
            return;
        }
    };
    let keys = match state.session_signing_keys(pubkey) {
        Ok(keys) => keys,
        Err(error) => {
            tracing::warn!(pubkey, %error, "profile retirement signer unavailable");
            return;
        }
    };
    let npub = crate::idref::npub(pubkey).unwrap_or_else(|| pubkey.to_string());
    let profile = crate::domain::Profile::agent(
        crate::domain::AgentRef::new(pubkey.to_string(), npub),
        session.agent_slug.clone(),
        state.host().clone(),
        state.owners().clone(),
    );
    match state
        .provider()
        .enqueue(&crate::domain::DomainEvent::Profile(profile), &keys)
        .await
    {
        Ok(_) => complete(state, pubkey),
        Err(error) => tracing::warn!(
            pubkey,
            error = %format!("{error:#}"),
            "reclaimed handle retirement profile remains due"
        ),
    }
}

fn complete(state: &Arc<DaemonState>, pubkey: &str) {
    if let Err(error) = state.with_store(|store| store.complete_profile_retirement(pubkey)) {
        tracing::error!(pubkey, %error, "profile retirement completion was not recorded");
    }
}
