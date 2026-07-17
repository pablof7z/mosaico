use crate::daemon::server::DaemonState;
use anyhow::{Context, Result};
use nostr_sdk::prelude::ToBech32;
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Reservation {
    pub(super) pubkey: String,
    pub(super) agent_nsec: String,
    pub(super) runtime_generation: u64,
    pub(super) reclaimed_pubkey: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_fresh(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    harness: &str,
    bundle: &str,
    transport: &str,
    root: &str,
    group: Option<&str>,
    session_name: Option<&str>,
) -> Result<Reservation> {
    let prepared = crate::daemon::server::prepare_session_identity(state, agent, session_name)?;
    reserve_prepared(
        state,
        prepared,
        &agent.slug,
        harness,
        bundle,
        transport,
        root,
        group,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_fresh_for_pubkey(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    harness: &str,
    bundle: &str,
    transport: &str,
    root: &str,
    group: Option<&str>,
    expected_pubkey: &str,
) -> Result<Reservation> {
    if agent.per_session_key {
        anyhow::bail!(
            "cannot fresh-launch per-session agent {:?} for existing pubkey {}; a native resume locator is required",
            agent.slug,
            expected_pubkey
        );
    }
    let configured_pubkey = agent
        .pubkey_hex()
        .context("durable agent has no configured pubkey")?;
    if configured_pubkey != expected_pubkey {
        anyhow::bail!(
            "configured durable pubkey {configured_pubkey} does not match addressed pubkey {expected_pubkey}"
        );
    }
    let prepared = crate::daemon::server::prepare_session_identity(state, agent, None)?;
    reserve_prepared(
        state,
        prepared,
        &agent.slug,
        harness,
        bundle,
        transport,
        root,
        group,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn reserve_resume(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    harness: &str,
    bundle: &str,
    transport: &str,
    root: &str,
    group: &str,
    native_resume: &str,
) -> Result<Reservation> {
    let pubkey = state
        .with_store(|store| {
            store.resolve_pubkey_by_locator(
                harness,
                crate::state::LOCATOR_NATIVE_RESUME,
                native_resume,
            )
        })?
        .with_context(|| {
            format!("no local pubkey owns {harness} resume locator {native_resume:?}")
        })?;
    let prepared = crate::daemon::server::load_session_identity(state, &pubkey, agent)?;
    reserve_prepared(
        state,
        prepared,
        &agent.slug,
        harness,
        bundle,
        transport,
        root,
        Some(group),
    )
}

#[allow(clippy::too_many_arguments)]
fn reserve_prepared(
    state: &Arc<DaemonState>,
    prepared: crate::daemon::server::PreparedIdentity,
    slug: &str,
    harness: &str,
    bundle: &str,
    transport: &str,
    root: &str,
    group: Option<&str>,
) -> Result<Reservation> {
    let agent_nsec = prepared
        .keys
        .secret_key()
        .to_bech32()
        .context("encoding the assigned agent session signer")?;
    let pubkey = prepared.identity.pubkey;
    let channel = match group.filter(|group| !group.is_empty()) {
        Some(group) => group.to_string(),
        None if state.per_session_rooms() => crate::util::session_room_id(&pubkey),
        None => root.to_string(),
    };
    let runtime_generation = state.with_store(|store| {
        store.reserve_session_with_facts(
            &crate::state::RegisterSession {
                pubkey: pubkey.clone(),
                observed_harness: harness.to_string(),
                agent_slug: slug.to_string(),
                channel_h: channel,
                child_pid: None,
                transcript_path: None,
                now: crate::util::now_secs(),
            },
            &crate::state::AdmittedRuntimeFacts {
                observed_harness: harness.to_string(),
                claimed_harness: String::new(),
                bundle: bundle.to_string(),
                transport: transport.to_string(),
                endpoint_provenance: "launch".to_string(),
            },
        )
    })?;
    Ok(Reservation {
        pubkey,
        agent_nsec,
        runtime_generation,
        reclaimed_pubkey: prepared.reclaimed_pubkey,
    })
}

pub(super) fn release(state: &Arc<DaemonState>, reservation: &Reservation) {
    if let Err(error) = state.with_store(|store| {
        store.mark_dead_if_generation(&reservation.pubkey, reservation.runtime_generation)
    }) {
        tracing::warn!(
            pubkey = %reservation.pubkey,
            runtime_generation = reservation.runtime_generation,
            %error,
            "failed to release pre-spawn runtime reservation"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_sdk::prelude::Keys;

    fn agent() -> crate::identity::AgentIdentity {
        crate::identity::AgentIdentity {
            slug: "codex".into(),
            keys: None,
            per_session_key: true,
            harness: "codex".into(),
            profile: None,
        }
    }

    #[tokio::test]
    async fn fresh_and_resumed_reservations_expose_the_same_assigned_signer() {
        let state = DaemonState::new_for_test().await;
        let agent = agent();
        let fresh = reserve_fresh(
            &state,
            &agent,
            "codex",
            "codex-pty",
            "pty",
            "root",
            None,
            None,
        )
        .unwrap();
        state
            .with_store(|store| {
                store.set_native_resume_locator(&fresh.pubkey, "codex", "native-1", 1)
            })
            .unwrap();
        release(&state, &fresh);

        let resumed = reserve_resume(
            &state,
            &agent,
            "codex",
            "codex-pty",
            "pty",
            "root",
            "root",
            "native-1",
        )
        .unwrap();

        assert_eq!(resumed.pubkey, fresh.pubkey);
        assert_eq!(resumed.agent_nsec, fresh.agent_nsec);
        assert_eq!(
            Keys::parse(&resumed.agent_nsec)
                .unwrap()
                .public_key()
                .to_hex(),
            resumed.pubkey
        );
    }

    #[tokio::test]
    async fn exact_fresh_launch_requires_a_matching_durable_pubkey() {
        let state = DaemonState::new_for_test().await;
        let per_session = agent();
        let error = match reserve_fresh_for_pubkey(
            &state,
            &per_session,
            "codex",
            "codex-pty",
            "pty",
            "root",
            Some("root"),
            "addressed",
        ) {
            Ok(_) => panic!("per-session identity unexpectedly fresh-launched for an old pubkey"),
            Err(error) => error,
        };
        assert!(error
            .to_string()
            .contains("native resume locator is required"));
        assert!(state
            .with_store(|store| store.get_session("addressed"))
            .unwrap()
            .is_none());

        let keys = Keys::generate();
        let pubkey = keys.public_key().to_hex();
        let durable = crate::identity::AgentIdentity {
            slug: "integrator".into(),
            keys: Some(keys),
            per_session_key: false,
            harness: "codex".into(),
            profile: None,
        };
        let reservation = reserve_fresh_for_pubkey(
            &state,
            &durable,
            "codex",
            "codex-pty",
            "pty",
            "root",
            Some("root"),
            &pubkey,
        )
        .unwrap();
        assert_eq!(reservation.pubkey, pubkey);
        release(&state, &reservation);
    }
}
