use super::model::*;
use crate::state::{Channel, Status};
use crate::util::relative_time;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct AgentWhoInput<'a> {
    pub(crate) roots: &'a [String],
    pub(crate) self_name: &'a str,
    pub(crate) self_pubkey: &'a str,
    pub(crate) local_host: &'a str,
    pub(crate) backend_pubkey: &'a str,
    pub(crate) now: u64,
    pub(crate) headless: bool,
    pub(crate) expanded_workspaces: &'a BTreeSet<String>,
}

pub(super) fn build_agent_who(
    aggregation: &crate::who_aggregation::WhoAggregation,
    input: AgentWhoInput<'_>,
) -> anyhow::Result<AgentWhoView> {
    let by_parent = channels_by_parent(aggregation.channels.clone());
    let workspaces = input
        .roots
        .iter()
        .map(|root| workspace_view(aggregation, &by_parent, root, &input))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(AgentWhoView {
        self_name: input.self_name.to_string(),
        self_host: input.local_host.to_string(),
        headless: input.headless,
        agents: available_agents(aggregation, input.local_host)?,
        workspaces,
    })
}

fn available_agents(
    aggregation: &crate::who_aggregation::WhoAggregation,
    local_host: &str,
) -> anyhow::Result<Vec<AgentCapabilityView>> {
    let mut grouped: BTreeMap<(String, String, String), BTreeSet<String>> = BTreeMap::new();
    for row in &aggregation.agents {
        let name = if row.host.is_empty() || row.host == local_host {
            row.slug.clone()
        } else {
            format!("{}@{}", row.slug, row.host)
        };
        let about = crate::agent_about::for_injection(&row.use_criteria);
        let workspace = aggregation.root_for_channel(&row.channel_h)?;
        grouped
            .entry((row.backend_pubkey.clone(), name, about))
            .or_default()
            .insert(workspace);
    }
    Ok(grouped
        .into_iter()
        .map(
            |((_backend, name, about), workspaces)| AgentCapabilityView {
                name,
                about,
                workspaces: workspaces.into_iter().collect(),
            },
        )
        .collect())
}

fn channels_by_parent(channels: Vec<Channel>) -> BTreeMap<String, Vec<Channel>> {
    let mut by_parent: BTreeMap<String, Vec<Channel>> = BTreeMap::new();
    for channel in channels
        .into_iter()
        .filter(|channel| !channel.is_archived())
    {
        by_parent
            .entry(channel.parent.clone())
            .or_default()
            .push(channel);
    }
    for children in by_parent.values_mut() {
        children.sort_by(|a, b| a.name.cmp(&b.name).then(a.channel_h.cmp(&b.channel_h)));
    }
    by_parent
}

fn workspace_view(
    aggregation: &crate::who_aggregation::WhoAggregation,
    by_parent: &BTreeMap<String, Vec<Channel>>,
    root: &str,
    input: &AgentWhoInput<'_>,
) -> anyhow::Result<WorkspaceView> {
    let meta = aggregation.channel(root);
    let members = member_views(aggregation, root, input);
    let expanded = input.expanded_workspaces.contains(root);
    let channels = if expanded {
        let mut seen = BTreeSet::from([root.to_string()]);
        by_parent
            .get(root)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|child| {
                channel_view(
                    aggregation,
                    by_parent,
                    root,
                    &child.channel_h,
                    input,
                    &mut seen,
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok(WorkspaceView {
        name: root.to_string(),
        channel: root.to_string(),
        path: {
            aggregation.root_for_channel(root)?;
            aggregation
                .workspace_path(root)
                .unwrap_or_default()
                .to_string()
        },
        about: meta
            .map(|channel| channel.about.clone())
            .unwrap_or_default(),
        member_count: members.len(),
        expanded,
        members: if expanded { members } else { Vec::new() },
        channels,
    })
}

fn channel_view(
    aggregation: &crate::who_aggregation::WhoAggregation,
    by_parent: &BTreeMap<String, Vec<Channel>>,
    workspace: &str,
    channel_h: &str,
    input: &AgentWhoInput<'_>,
    seen: &mut BTreeSet<String>,
) -> anyhow::Result<ChannelView> {
    if !seen.insert(channel_h.to_string()) {
        return Ok(empty_channel(workspace, channel_h));
    }
    let meta = aggregation.channel(channel_h);
    let name = aggregation.channel_name(channel_h).to_string();
    let members = member_views(aggregation, channel_h, input);
    let expanded = aggregation.is_member(channel_h, input.self_pubkey);
    let children = if expanded {
        by_parent
            .get(channel_h)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|child| {
                channel_view(
                    aggregation,
                    by_parent,
                    workspace,
                    &child.channel_h,
                    input,
                    seen,
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    Ok(ChannelView {
        name,
        id: aggregation.full_channel_ref(channel_h)?,
        about: meta
            .map(|channel| channel.about.clone())
            .unwrap_or_default(),
        member_count: members.len(),
        expanded,
        members: if expanded { members } else { Vec::new() },
        children,
    })
}

fn empty_channel(workspace: &str, channel_h: &str) -> ChannelView {
    ChannelView {
        name: channel_h.to_string(),
        id: format!("{workspace}.{channel_h}"),
        about: String::new(),
        member_count: 0,
        expanded: false,
        members: Vec::new(),
        children: Vec::new(),
    }
}

fn member_views(
    aggregation: &crate::who_aggregation::WhoAggregation,
    channel: &str,
    input: &AgentWhoInput<'_>,
) -> Vec<MemberView> {
    let backend_pubkeys = aggregation
        .agents
        .iter()
        .map(|row| row.backend_pubkey.clone())
        .collect::<BTreeSet<_>>();
    let statuses = aggregation
        .statuses_for(channel)
        .iter()
        .map(|status| (status.pubkey.clone(), status))
        .collect::<BTreeMap<_, _>>();
    aggregation
        .members_for(channel)
        .iter()
        .filter(|member| {
            member.pubkey != input.backend_pubkey
                && !backend_pubkeys.contains(&member.pubkey)
                && !aggregation
                    .profile(&member.pubkey)
                    .is_some_and(|profile| profile.is_backend)
        })
        .map(|member| {
            member_view(
                aggregation,
                &member.pubkey,
                statuses.get(&member.pubkey).copied(),
                input,
            )
        })
        .collect()
}

fn member_view(
    aggregation: &crate::who_aggregation::WhoAggregation,
    pubkey: &str,
    status: Option<&Status>,
    input: &AgentWhoInput<'_>,
) -> MemberView {
    let profile = aggregation.profile(pubkey);
    let is_agent = pubkey == input.self_pubkey
        || status.is_some()
        || profile.is_some_and(|row| !row.agent_slug.is_empty());
    let name = if pubkey == input.self_pubkey {
        input.self_name.to_string()
    } else {
        aggregation.pubkey_ref(pubkey, input.local_host)
    };
    let (state, text, since) = match status {
        Some(row) => {
            let presence = aggregation.public_presence(pubkey, row);
            (
                presence.state,
                presence.text(),
                relative_time(presence.state_since, input.now),
            )
        }
        None => (
            crate::session_state::SessionState::Offline,
            String::new(),
            "unknown".to_string(),
        ),
    };
    MemberView {
        kind: if is_agent {
            MemberKind::Agent
        } else {
            MemberKind::Human
        },
        name,
        state,
        status: text,
        since,
    }
}
