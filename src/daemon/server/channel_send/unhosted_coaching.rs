use super::super::DaemonState;
use super::coaching::{self, CoachingNotice};
use anyhow::Result;
use std::sync::Arc;

const CODE: &str = "unhosted_return_path";

pub(super) fn notices(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    channel: &str,
    expected_authors: &[String],
    wait_intent: bool,
    shown_at: u64,
) -> Vec<CoachingNotice> {
    match maybe_warn(
        state,
        session,
        channel,
        expected_authors,
        wait_intent,
        shown_at,
    ) {
        Ok(notice) => notice.into_iter().collect(),
        Err(error) => {
            tracing::error!(%error, "unhosted return-path coaching unavailable");
            Vec::new()
        }
    }
}

pub(super) fn maybe_warn(
    state: &Arc<DaemonState>,
    session: &crate::state::Session,
    channel: &str,
    expected_authors: &[String],
    wait_intent: bool,
    shown_at: u64,
) -> Result<Option<CoachingNotice>> {
    if !session.admitted_transport.is_empty()
        || expected_authors.is_empty()
        || wait_intent
        || state.has_matching_active_wait(session, channel, expected_authors)
    {
        return Ok(None);
    }
    let claimed = state.with_store(|store| {
        store.claim_session_coaching(&session.pubkey, session.runtime_generation, CODE, shown_at)
    })?;
    Ok(claimed.then(coaching::unhosted_no_return_path))
}
