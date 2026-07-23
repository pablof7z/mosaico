//! Canonical projection from lifecycle or relay facts to public presence.

use crate::session_state::{semantic_change_at, SessionState};
use crate::state::{Session, Status, Store};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicPresence {
    pub(crate) state: SessionState,
    pub(crate) state_since: u64,
    pub(crate) title: String,
    pub(crate) activity: String,
    /// Owner-observed runtime time locally; lease-observation time remotely.
    pub(crate) observed_at: u64,
}

impl PublicPresence {
    pub(crate) fn text(&self) -> String {
        if self.state.is_working() && !self.activity.trim().is_empty() {
            self.activity.trim().to_string()
        } else {
            self.title.trim().to_string()
        }
    }
}

/// Project authoritative local lifecycle. Lease freshness never overrides an
/// owning daemon's knowledge that its runtime is still running.
pub(crate) fn local(
    store: &Store,
    session: &Session,
    published: Option<&Status>,
) -> PublicPresence {
    let state = SessionState::classify(
        session.is_running(),
        session.is_working(),
        crate::session_host::session_has_live_delivery_path(store, session),
    );
    let matching = published.filter(|status| status.state == state);
    let title = if session.title.trim().is_empty() {
        published
            .map(|status| status.title.clone())
            .unwrap_or_default()
    } else {
        session.title.clone()
    };
    let activity = matching
        .filter(|_| state.is_working())
        .map(|status| status.activity.clone())
        .unwrap_or_default();
    PublicPresence {
        state,
        state_since: local_transition_hint(session, state),
        title,
        activity,
        observed_at: session.last_seen,
    }
}

pub(crate) fn publication(
    store: &Store,
    session: &Session,
) -> crate::reconcile::PresenceProjection {
    let published = store
        .get_status(&session.pubkey, &session.channel_h)
        .ok()
        .flatten();
    let presence = local(store, session, published.as_ref());
    let mut channels = store
        .list_session_routes(&session.pubkey)
        .unwrap_or_default()
        .into_iter()
        .map(|(channel, _)| channel)
        .filter(|channel| !store.is_archived_channel(channel).unwrap_or(false))
        .collect::<BTreeSet<_>>();
    if !session.channel_h.is_empty()
        && !store
            .is_archived_channel(&session.channel_h)
            .unwrap_or(false)
    {
        channels.insert(session.channel_h.clone());
    }
    crate::reconcile::PresenceProjection {
        channels,
        state: presence.state,
        state_since: presence.state_since,
        title: presence.title,
    }
}

/// Project a signed remote lease. Expiry is an observer fact and therefore can
/// make a remote session offline without rewriting the owner's semantic state.
pub(crate) fn remote(status: &Status, now: u64) -> PublicPresence {
    observed(
        status.state,
        status.state_since,
        &status.title,
        &status.activity,
        status.last_seen,
        Some(status.expiration),
        now,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn observed(
    reported_state: SessionState,
    state_since: u64,
    title: &str,
    activity: &str,
    observed_at: u64,
    expiration: Option<u64>,
    now: u64,
) -> PublicPresence {
    let live = expiration.is_none_or(|expires_at| expires_at >= now);
    let state = reported_state.observed(live);
    PublicPresence {
        state,
        state_since: expiration
            .map(|expires_at| semantic_change_at(reported_state, state_since, expires_at, now))
            .unwrap_or(state_since),
        title: title.to_string(),
        activity: if state.is_working() {
            activity.to_string()
        } else {
            String::new()
        },
        observed_at,
    }
}

fn local_transition_hint(session: &Session, state: SessionState) -> u64 {
    if session.state_changed_at > 0 {
        return session.state_changed_at;
    }
    match state {
        SessionState::Working => session.turn_started_at,
        SessionState::Offline => session.stopped_at,
        SessionState::Idle | SessionState::Suspended => session.created_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_expiry_changes_state_time_without_using_observation_time() {
        let status = Status {
            pubkey: "peer".into(),
            channel_h: "room".into(),
            slug: "peer".into(),
            title: "Task".into(),
            activity: "Working".into(),
            state: SessionState::Working,
            state_since: 90,
            last_seen: 115,
            updated_at: 90,
            expiration: 120,
        };
        let projected = remote(&status, 121);
        assert_eq!(projected.state, SessionState::Offline);
        assert_eq!(projected.state_since, 121);
        assert_eq!(projected.observed_at, 115);
        assert!(projected.activity.is_empty());
    }
}
