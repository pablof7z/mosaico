use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::read;
use crate::state::Store;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::fabric_context) struct StatusCap {
    pub(in crate::fabric_context) pubkey: String,
    #[serde(default)]
    pub(in crate::fabric_context) host: String,
    #[serde(default)]
    pub(in crate::fabric_context) slug: String,
    pub(in crate::fabric_context) state: crate::session_state::SessionState,
    pub(in crate::fabric_context) activity: String,
    pub(in crate::fabric_context) title: String,
    /// Latest semantic record change, distinct from the lifecycle transition.
    #[serde(default)]
    pub(in crate::fabric_context) changed_at: u64,
    pub(in crate::fabric_context) state_since: u64,
    pub(in crate::fabric_context) observed_at: u64,
    /// Absent for lifecycle-authoritative local sessions.
    pub(in crate::fabric_context) expiration: Option<u64>,
    #[serde(default)]
    pub(in crate::fabric_context) native_failure: Option<NativeFailureCap>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::fabric_context) struct NativeFailureCap {
    pub(in crate::fabric_context) outcome: String,
    pub(in crate::fabric_context) message: String,
    pub(in crate::fabric_context) finished_at: u64,
}

pub(super) fn status_caps(
    store: &Store,
    channel: &str,
    local_host: &str,
    refs: &mut BTreeMap<String, String>,
    agent_slugs: &mut BTreeMap<String, String>,
    backend: &mut BTreeSet<String>,
    has_handle: &mut BTreeSet<String>,
) -> Vec<StatusCap> {
    let mut rows = store
        .live_status_for_channel(channel, 0)
        .unwrap_or_default()
        .into_iter()
        .map(|status| {
            read::resolve_pubkey(
                store,
                &status.pubkey,
                local_host,
                refs,
                agent_slugs,
                backend,
                has_handle,
            );
            let local_session = store
                .get_session(&status.pubkey)
                .ok()
                .flatten()
                .filter(|session| session.is_running());
            let native_failure = local_session
                .as_ref()
                .and_then(|session| native_failure(store, session));
            let local = local_session
                .as_ref()
                .map(|session| crate::session_presence::local(store, session, Some(&status)));
            StatusCap {
                host: read::profile_host(store, &status.pubkey),
                slug: status.slug,
                pubkey: status.pubkey,
                state: local.as_ref().map_or(status.state, |row| row.state),
                activity: local
                    .as_ref()
                    .map_or(status.activity, |row| row.activity.clone()),
                title: local.as_ref().map_or(status.title, |row| row.title.clone()),
                changed_at: local.as_ref().map_or(status.updated_at, |row| {
                    status.updated_at.max(row.state_since)
                }),
                state_since: local
                    .as_ref()
                    .map_or(status.state_since, |row| row.state_since),
                observed_at: local
                    .as_ref()
                    .map_or(status.last_seen, |row| row.observed_at),
                expiration: local.is_none().then_some(status.expiration),
                native_failure,
            }
        })
        .collect::<Vec<_>>();
    for session in store.list_running_sessions().unwrap_or_default() {
        let routed = session.channel_h == channel
            || store
                .has_session_route(&session.pubkey, channel)
                .unwrap_or(false);
        if !routed || rows.iter().any(|row| row.pubkey == session.pubkey) {
            continue;
        }
        read::resolve_pubkey(
            store,
            &session.pubkey,
            local_host,
            refs,
            agent_slugs,
            backend,
            has_handle,
        );
        let presence = crate::session_presence::local(store, &session, None);
        let native_failure = native_failure(store, &session);
        let slug = store
            .session_identity(&session.pubkey)
            .ok()
            .flatten()
            .map(|identity| identity.display_slug())
            .unwrap_or_else(|| session.agent_slug.clone());
        rows.push(StatusCap {
            pubkey: session.pubkey,
            host: local_host.to_string(),
            slug,
            state: presence.state,
            activity: presence.activity,
            title: presence.title,
            changed_at: presence.state_since,
            state_since: presence.state_since,
            observed_at: presence.observed_at,
            expiration: None,
            native_failure,
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.changed_at));
    rows
}

fn native_failure(store: &Store, session: &crate::state::Session) -> Option<NativeFailureCap> {
    store
        .latest_native_turn_attempt(&session.pubkey, session.runtime_generation)
        .ok()
        .flatten()
        .filter(|attempt| attempt.outcome.is_failure())
        .map(|attempt| NativeFailureCap {
            outcome: attempt.outcome.as_str().to_string(),
            message: attempt.error_message,
            finished_at: attempt.finished_at,
        })
}
