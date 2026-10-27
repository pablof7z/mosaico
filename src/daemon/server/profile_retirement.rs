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
    match publish(state, pubkey).await {
        Ok(()) => complete(state, pubkey),
        Err(error) => record_failure(state, pubkey, &error),
    }
}

/// Publish one retirement. Any failure, local (read, signer) or relay, takes
/// the same backoff: a missing signer can be permanent.
async fn publish(state: &Arc<DaemonState>, pubkey: &str) -> std::result::Result<(), String> {
    let session = match state.with_store(|store| store.get_session(pubkey)) {
        Ok(Some(session)) => session,
        // Nothing local signs for this pubkey any more.
        Ok(None) => return Ok(()),
        Err(error) => return Err(format!("profile retirement read failed: {error:#}")),
    };
    let keys = state
        .session_signing_keys(pubkey)
        .map_err(|error| format!("profile retirement signer unavailable: {error:#}"))?;
    let npub = crate::idref::npub(pubkey).unwrap_or_else(|| pubkey.to_string());
    let profile = crate::domain::Profile::agent(
        crate::domain::AgentRef::new(pubkey.to_string(), npub),
        session.agent_slug.clone(),
        state.host().clone(),
        state.owners().clone(),
    );
    state
        .provider()
        .enqueue(&crate::domain::DomainEvent::Profile(profile), &keys)
        .await
        .map(|_| ())
        .map_err(|error| format!("{error:#}"))
}

fn record_failure(state: &Arc<DaemonState>, pubkey: &str, error: &str) {
    let failures = state
        .with_store(|store| store.record_profile_retirement_failure(pubkey, now_secs()))
        .unwrap_or(0);
    if warns_on_failure(failures) {
        tracing::warn!(pubkey, %error, "reclaimed handle retirement profile remains due");
    } else {
        tracing::debug!(pubkey, failures, %error, "retirement retry failed");
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
    use super::*;

    /// A retirement that cannot even be signed (here: no signer bound) still
    /// backs off like a relay failure instead of retrying every tick forever.
    #[tokio::test]
    async fn a_local_retirement_failure_backs_off_too() {
        let state = DaemonState::new_for_test().await;
        state.with_store(|store| {
            store
                .reserve_session_with_facts(
                    &crate::state::RegisterSession {
                        pubkey: "unsignable".into(),
                        observed_harness: "codex".into(),
                        agent_slug: "codex".into(),
                        launch_channel_h: "room".into(),
                        work_root: "room".into(),
                        child_pid: None,
                        now: 1,
                    },
                    &crate::state::AdmittedRuntimeFacts {
                        observed_harness: "codex".into(),
                        claimed_harness: String::new(),
                        preset: String::new(),
                        transport: "pty".into(),
                        endpoint_provenance: "launch".into(),
                    },
                )
                .unwrap();
            store.schedule_profile_retirement("unsignable", 1).unwrap();
        });

        retry_due(&state).await;

        state.with_store(|store| {
            assert_eq!(
                store.profile_retirement_attempts("unsignable").unwrap(),
                Some(1)
            );
            assert!(store
                .list_due_profile_retirements(now_secs())
                .unwrap()
                .is_empty());
        });
    }

    #[test]
    fn only_the_first_failure_of_a_retirement_warns() {
        assert!(warns_on_failure(1));
        assert!((2..=50).all(|failures| !warns_on_failure(failures)));
    }
}
