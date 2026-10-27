//! Durable retirement of a reclaimed handle's previous owner.
//!
//! A reclaimed handle's previous owner keeps advertising the handle in its
//! kind:0 until a retirement profile reaches the relay. Session start only
//! records that work durably, so the hook path stays fail-open on a wedged
//! relay. A dedicated loop publishes due work, backing a failing retirement
//! off exponentially (5 s doubling to 10 min) until the relay acknowledges it,
//! across reconnects and restarts.

use super::*;

/// Record the retirement; the retirement loop publishes it.
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
    Ok(())
}

/// Retry every retirement the relay has not yet acknowledged.
pub(in crate::daemon::server) async fn retry_due(state: &Arc<DaemonState>) {
    let due = state
        .with_store(|store| store.list_due_profile_retirements(now_secs()))
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
        Err(error) => {
            let failures = state
                .with_store(|store| store.record_profile_retirement_failure(pubkey, now_secs()))
                .unwrap_or(0);
            let error = format!("{error:#}");
            if warns_on_failure(failures) {
                tracing::warn!(pubkey, %error, "reclaimed handle retirement profile remains due");
            } else {
                tracing::debug!(pubkey, failures, %error, "retirement retry failed");
            }
        }
    }
}

/// Only the first failure of a retirement is a warning; its retries, which
/// can continue for as long as a relay stays unreachable, log at debug.
fn warns_on_failure(failures: u32) -> bool {
    failures <= 1
}

fn complete(state: &Arc<DaemonState>, pubkey: &str) {
    if let Err(error) = state.with_store(|store| store.complete_profile_retirement(pubkey)) {
        tracing::error!(pubkey, %error, "profile retirement completion was not recorded");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_first_failure_of_a_retirement_warns() {
        assert!(super::warns_on_failure(1));
        assert!((2..=50).all(|failures| !super::warns_on_failure(failures)));
    }
}
