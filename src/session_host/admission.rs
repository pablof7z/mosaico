use crate::daemon::server::DaemonState;
use anyhow::{Context, Result};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct Reservation {
    pub(super) pubkey: String,
    pub(super) runtime_generation: u64,
    pub(super) reclaimed_pubkey: Option<String>,
}

pub(super) fn reserve_fresh(
    state: &Arc<DaemonState>,
    slug: &str,
    harness: &str,
    root: &str,
    group: Option<&str>,
    session_name: Option<&str>,
) -> Result<Reservation> {
    let agent = crate::identity::load(&crate::config::mosaico_home(), slug)?;
    let prepared = crate::daemon::server::prepare_session_identity(state, &agent, session_name)?;
    reserve_prepared(
        state,
        prepared.identity.pubkey,
        prepared.reclaimed_pubkey,
        slug,
        harness,
        root,
        group,
    )
}

pub(super) fn reserve_resume(
    state: &Arc<DaemonState>,
    slug: &str,
    harness: &str,
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
    let agent = crate::identity::load(&crate::config::mosaico_home(), slug)?;
    let session = state
        .with_store(|store| store.get_session(&pubkey))?
        .with_context(|| format!("resume pubkey {pubkey} has no local runtime projection"))?;
    crate::daemon::server::validate_live_session_identity(state, &session, &agent)?;
    reserve_prepared(state, pubkey, None, slug, harness, root, Some(group))
}

fn reserve_prepared(
    state: &Arc<DaemonState>,
    pubkey: String,
    reclaimed_pubkey: Option<String>,
    slug: &str,
    harness: &str,
    root: &str,
    group: Option<&str>,
) -> Result<Reservation> {
    let channel = match group.filter(|group| !group.is_empty()) {
        Some(group) => group.to_string(),
        None if state.per_session_rooms() => crate::util::session_room_id(&pubkey),
        None => root.to_string(),
    };
    let runtime_generation = state.with_store(|store| {
        store.reserve_session(&crate::state::RegisterSession {
            pubkey: pubkey.clone(),
            harness: harness.to_string(),
            agent_slug: slug.to_string(),
            channel_h: channel,
            child_pid: None,
            transcript_path: None,
            now: crate::util::now_secs(),
        })
    })?;
    Ok(Reservation {
        pubkey,
        runtime_generation,
        reclaimed_pubkey,
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
