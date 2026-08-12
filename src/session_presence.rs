//! Canonical projection from lifecycle or relay facts to public presence.

use crate::session_state::SessionState;
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
    let route_rows = store
        .list_session_routes(&session.pubkey)
        .unwrap_or_default();
    let published = route_rows
        .iter()
        .find_map(|(channel, _)| store.get_status(&session.pubkey, channel).ok().flatten());
    let presence = local(store, session, published.as_ref());
    let channels = route_rows
        .into_iter()
        .map(|(channel, _)| channel)
        .filter(|channel| {
            store
                .get_session_standing(&session.pubkey, channel)
                .is_ok_and(|standing| {
                    standing.is_some_and(|standing| {
                        standing.state == crate::state::StandingState::Member
                            && standing.session_lifecycle_epoch == session.lifecycle_epoch
                    })
                })
        })
        .filter(|channel| !store.is_archived_channel(channel).unwrap_or(false))
        .collect::<BTreeSet<_>>();
    crate::reconcile::PresenceProjection {
        channels,
        branch: crate::worktree_branch::for_root(store, &session.work_root),
        state: presence.state,
        state_since: presence.state_since,
        title: presence.title,
    }
}

/// Project a current signed remote status Row. NMP has already applied NIP-40
/// expiry; absence, rather than a second Mosaico clock, represents offline.
pub(crate) fn remote(status: &Status) -> PublicPresence {
    PublicPresence {
        state: status.state,
        state_since: status.state_since,
        title: status.title.clone(),
        activity: if status.state.is_working() {
            status.activity.clone()
        } else {
            String::new()
        },
        observed_at: status.last_seen,
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
    fn publication_exposes_only_relay_confirmed_membership_routes() {
        let store = Store::open_memory().unwrap();
        let generation = store
            .reserve_session_with_facts(
                &crate::state::RegisterSession {
                    pubkey: "pk".into(),
                    observed_harness: "codex".into(),
                    agent_slug: "agent".into(),
                    launch_channel_h: "room".into(),
                    work_root: "room".into(),
                    child_pid: None,
                    now: 1,
                },
                &crate::state::AdmittedRuntimeFacts {
                    observed_harness: "codex".into(),
                    claimed_harness: String::new(),
                    bundle: "codex-pty".into(),
                    transport: "pty".into(),
                    endpoint_provenance: "launch".into(),
                },
            )
            .unwrap();
        let session = store.get_session("pk").unwrap().unwrap();

        assert!(store.has_session_route("pk", "room").unwrap());
        assert!(publication(&store, &session).channels.is_empty());

        store
            .commit_confirmed_session_admission(
                "pk",
                "room",
                generation,
                session.lifecycle_epoch,
                2,
            )
            .unwrap();
        assert_eq!(
            publication(&store, &session).channels,
            BTreeSet::from(["room".to_string()])
        );
    }

    #[test]
    fn current_remote_row_preserves_the_reported_semantic_state() {
        let status = Status {
            pubkey: "peer".into(),
            channel_h: "room".into(),
            slug: "peer".into(),
            title: "Task".into(),
            activity: "Working".into(),
            workspace: "mosaico".into(),
            branch: "feat/context".into(),
            state: SessionState::Working,
            state_since: 90,
            last_seen: 115,
            updated_at: 90,
            expiration: 120,
        };
        let projected = remote(&status);
        assert_eq!(projected.state, SessionState::Working);
        assert_eq!(projected.state_since, 90);
        assert_eq!(projected.observed_at, 115);
        assert_eq!(projected.activity, "Working");
    }
}
