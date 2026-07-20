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
        let existing = state
            .with_store(|store| store.get_session(expected_pubkey))?
            .with_context(|| {
                format!("cannot fresh-relaunch unknown per-session pubkey {expected_pubkey}")
            })?;
        if existing.agent_slug != agent.slug || existing.observed_harness != harness {
            anyhow::bail!(
                "cannot fresh-relaunch per-session pubkey {expected_pubkey}: persisted agent/harness ({}/{}) does not match requested ({}/{harness})",
                existing.agent_slug,
                existing.observed_harness,
                agent.slug,
            );
        }
        if state
            .with_store(|store| store.native_resume_locator(expected_pubkey, harness))?
            .is_some()
        {
            anyhow::bail!(
                "cannot fresh-relaunch per-session pubkey {expected_pubkey}: native resume is available"
            );
        }
        if !state.with_store(|store| store.session_can_fresh_relaunch_exact(expected_pubkey))? {
            anyhow::bail!(
                "cannot fresh-relaunch per-session pubkey {expected_pubkey}: exact relaunch requires a stopped, non-revoked session"
            );
        }
        let prepared = crate::daemon::server::load_session_identity(state, expected_pubkey, agent)?;
        return reserve_prepared(
            state,
            prepared,
            &agent.slug,
            harness,
            bundle,
            transport,
            root,
            group,
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
pub(super) fn reserve_resume_exact(
    state: &Arc<DaemonState>,
    agent: &crate::identity::AgentIdentity,
    pubkey: &str,
    slug: &str,
    harness: &str,
    bundle: &str,
    transport: &str,
    root: &str,
    group: &str,
) -> Result<Reservation> {
    let prepared = crate::daemon::server::load_session_identity(state, pubkey, agent)?;
    reserve_prepared(
        state,
        prepared,
        slug,
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
        store.mark_runtime_stopped_if_generation(
            &reservation.pubkey,
            reservation.runtime_generation,
            crate::state::StopReason::Unknown,
            crate::util::now_secs(),
        )
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
#[path = "admission/tests.rs"]
mod tests;
