use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{read, ChannelCap, SummaryCap};
use crate::state::Store;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::fabric_context) struct WorkspaceCap {
    pub(in crate::fabric_context) summary: SummaryCap,
    pub(in crate::fabric_context) channels: Vec<ChannelCap>,
}

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
}

pub(super) fn workspace_caps(
    store: &Store,
    current_root: &str,
) -> anyhow::Result<Vec<WorkspaceCap>> {
    let mut by_root: BTreeMap<String, Vec<ChannelCap>> = BTreeMap::new();
    for channel in store
        .list_channels()?
        .into_iter()
        .filter(|channel| !channel.is_archived())
    {
        let root = crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(&channel.channel_h)?;
        if root == current_root {
            continue;
        }
        let summary = read::channel_summary(store, &channel.channel_h);
        by_root.entry(root).or_default().push(ChannelCap {
            h: channel.channel_h,
            name: summary.name,
            reference: summary.channel,
            about: summary.about,
            subchannels: Vec::new(),
        });
    }
    Ok(by_root
        .into_iter()
        .map(|(root, channels)| WorkspaceCap {
            summary: read::workspace_summary(store, &root),
            channels,
        })
        .collect())
}

pub(super) fn capture_statuses(
    store: &Store,
    local_host: &str,
    workspaces: &[WorkspaceCap],
    statuses: &mut BTreeMap<String, Vec<StatusCap>>,
    refs: &mut BTreeMap<String, String>,
    agent_slugs: &mut BTreeMap<String, String>,
    backend: &mut BTreeSet<String>,
) {
    for channel in workspaces.iter().flat_map(|workspace| &workspace.channels) {
        if statuses.contains_key(&channel.h) {
            continue;
        }
        let captured = status_caps(store, &channel.h, local_host, refs, agent_slugs, backend);
        statuses.insert(channel.h.clone(), captured);
    }
}

pub(super) fn status_caps(
    store: &Store,
    channel: &str,
    local_host: &str,
    refs: &mut BTreeMap<String, String>,
    agent_slugs: &mut BTreeMap<String, String>,
    backend: &mut BTreeSet<String>,
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
            );
            let local = store
                .get_session(&status.pubkey)
                .ok()
                .flatten()
                .filter(|session| session.is_running())
                .map(|session| crate::session_presence::local(store, &session, Some(&status)));
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
        );
        let presence = crate::session_presence::local(store, &session, None);
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
        });
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.changed_at));
    rows
}
