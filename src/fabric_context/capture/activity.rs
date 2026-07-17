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
    pub(in crate::fabric_context) last_seen: u64,
    pub(in crate::fabric_context) updated_at: u64,
    pub(in crate::fabric_context) expiration: u64,
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
    store
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
            StatusCap {
                host: read::profile_host(store, &status.pubkey),
                slug: status.slug,
                pubkey: status.pubkey,
                state: status.state,
                activity: status.activity,
                title: status.title,
                last_seen: status.last_seen,
                updated_at: status.updated_at,
                expiration: status.expiration,
            }
        })
        .collect()
}
