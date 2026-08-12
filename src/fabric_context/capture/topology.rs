use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{AgentCap, ChannelCap, HostCap, SummaryCap};
use crate::state::{Profile, Store};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::fabric_context) struct WorkspaceCap {
    pub(in crate::fabric_context) summary: SummaryCap,
    pub(in crate::fabric_context) hosts: Vec<String>,
    pub(in crate::fabric_context) updated_at: u64,
    pub(in crate::fabric_context) channels: Vec<ChannelCap>,
}

pub(super) fn capture(store: &Store) -> anyhow::Result<(Vec<HostCap>, Vec<WorkspaceCap>)> {
    let latest_message_at = store.latest_message_at_by_channel()?;
    let channels = store
        .list_channels()?
        .into_iter()
        .filter(|channel| !channel.is_archived())
        .collect::<Vec<_>>();
    let mut roots = channels
        .iter()
        .filter(|channel| channel.parent.is_empty())
        .map(|channel| channel.channel_h.clone())
        .collect::<BTreeSet<_>>();
    roots.extend(
        store
            .list_workspace_bindings()?
            .into_iter()
            .map(|binding| binding.channel_h),
    );

    let mut channels_by_root = BTreeMap::<String, Vec<ChannelCap>>::new();
    for channel in &channels {
        // A channel whose parent metadata has not arrived yet cannot be placed
        // under a root. That is an ordinary transient — you are a member of a
        // child but not of its parent, so the parent's kind:39000 never lands —
        // and it must not take the whole topology down with it. Skip the one
        // channel; it appears once its ancestry resolves.
        let root = match crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(&channel.channel_h)
        {
            Ok(root) => root,
            Err(error) => {
                tracing::debug!(
                    channel = %channel.channel_h,
                    parent = %channel.parent,
                    error = %error,
                    "topology: skipping channel with unresolved ancestry"
                );
                continue;
            }
        };
        let reference = crate::channel_ref::full_channel_ref(store, &channel.channel_h);
        if reference.is_empty() {
            tracing::debug!(
                channel = %channel.channel_h,
                "topology: skipping channel without a complete public path"
            );
            continue;
        }
        channels_by_root.entry(root).or_default().push(ChannelCap {
            h: channel.channel_h.clone(),
            reference,
            about: channel.about.clone(),
            updated_at: channel.updated_at,
            latest_message_at: latest_message_at.get(&channel.channel_h).copied(),
        });
    }
    for rows in channels_by_root.values_mut() {
        rows.sort_by(|left, right| left.reference.cmp(&right.reference));
    }

    let profiles = store.list_backend_profiles()?;
    let workspace_hosts = workspace_hosts(store, &roots, &profiles);
    let workspaces = roots
        .into_iter()
        .map(|root| {
            let meta = channels.iter().find(|channel| channel.channel_h == root);
            WorkspaceCap {
                summary: SummaryCap {
                    name: root.clone(),
                    channel: crate::channel_ref::format_channel_ref(&root, &[]),
                    about: meta
                        .map(|channel| channel.about.clone())
                        .unwrap_or_default(),
                },
                hosts: workspace_hosts
                    .get(&root)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                updated_at: profiles
                    .iter()
                    .filter(|profile| profile_qualifies(store, &root, profile))
                    .map(|profile| profile.updated_at)
                    .max()
                    .unwrap_or_default(),
                channels: channels_by_root.remove(&root).unwrap_or_default(),
            }
        })
        .collect();
    let hosts = host_caps(&workspace_hosts, profiles);
    Ok((hosts, workspaces))
}

fn workspace_hosts(
    store: &Store,
    roots: &BTreeSet<String>,
    profiles: &[Profile],
) -> BTreeMap<String, BTreeSet<String>> {
    roots
        .iter()
        .map(|root| {
            let hosts = profiles
                .iter()
                .filter(|profile| profile_qualifies(store, root, profile))
                .map(|profile| profile.host.clone())
                .collect();
            (root.clone(), hosts)
        })
        .collect()
}

fn profile_qualifies(store: &Store, root: &str, profile: &Profile) -> bool {
    !profile.host.is_empty()
        && profile.workspaces.iter().any(|workspace| workspace == root)
        && store
            .is_channel_admin(root, &profile.pubkey)
            .unwrap_or(false)
}

fn host_caps(
    workspace_hosts: &BTreeMap<String, BTreeSet<String>>,
    profiles: Vec<Profile>,
) -> Vec<HostCap> {
    let advertised = workspace_hosts
        .values()
        .flat_map(|hosts| hosts.iter().cloned())
        .collect::<BTreeSet<_>>();
    let mut grouped = BTreeMap::<String, BTreeMap<String, AgentCap>>::new();
    for profile in profiles {
        if !advertised.contains(&profile.host) {
            continue;
        }
        let host_agents = grouped.entry(profile.host.clone()).or_default();
        for (slug, about) in profile.agents {
            let reference = format!("{slug}@{}", profile.host);
            host_agents
                .entry(reference.clone())
                .and_modify(|agent| {
                    if profile.updated_at >= agent.created_at {
                        agent.about = about.clone();
                        agent.created_at = profile.updated_at;
                    }
                })
                .or_insert(AgentCap {
                    reference,
                    about,
                    created_at: profile.updated_at,
                });
        }
    }
    grouped
        .into_iter()
        .map(|(name, agents)| HostCap {
            roots: workspace_hosts
                .iter()
                .filter(|(_, hosts)| hosts.contains(&name))
                .map(|(root, _)| root.clone())
                .collect(),
            name,
            agents: agents.into_values().collect(),
        })
        .collect()
}
