use super::model::*;
use crate::state::{Channel, Status, Store};
use crate::util::relative_time;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct AgentWhoInput<'a> {
    pub(crate) roots: &'a [String],
    pub(crate) self_name: &'a str,
    pub(crate) self_pubkey: &'a str,
    pub(crate) local_host: &'a str,
    pub(crate) backend_pubkey: &'a str,
    pub(crate) now: u64,
    pub(crate) expanded_workspaces: &'a BTreeSet<String>,
}

pub(super) fn build_agent_who(store: &Store, input: AgentWhoInput<'_>) -> AgentWhoView {
    let channels = store.list_channels().unwrap_or_default();
    let by_parent = channels_by_parent(channels);
    let workspaces = input
        .roots
        .iter()
        .map(|root| workspace_view(store, &by_parent, root, &input))
        .collect();
    AgentWhoView {
        self_name: input.self_name.to_string(),
        self_host: input.local_host.to_string(),
        agents: available_agents(store, input.local_host),
        workspaces,
    }
}

fn available_agents(store: &Store, local_host: &str) -> Vec<AvailableAgent> {
    let mut grouped: BTreeMap<(String, String, String), BTreeSet<String>> = BTreeMap::new();
    for row in store.list_agent_roster().unwrap_or_default() {
        let name = if row.host.is_empty() || row.host == local_host {
            row.slug
        } else {
            format!("{}@{}", row.slug, row.host)
        };
        grouped
            .entry((row.backend_pubkey, name, row.use_criteria))
            .or_default()
            .insert(
                store
                    .root_channel_of(&row.channel_h)
                    .ok()
                    .flatten()
                    .unwrap_or(row.channel_h),
            );
    }
    grouped
        .into_iter()
        .map(|((_backend, name, about), workspaces)| AvailableAgent {
            name,
            about,
            workspaces: workspaces.into_iter().collect(),
        })
        .collect()
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
    store: &Store,
    by_parent: &BTreeMap<String, Vec<Channel>>,
    root: &str,
    input: &AgentWhoInput<'_>,
) -> WorkspaceView {
    let meta = store.get_channel(root).ok().flatten();
    let members = member_views(store, root, input);
    let expanded = input.expanded_workspaces.contains(root);
    let channels = if expanded {
        let mut seen = BTreeSet::from([root.to_string()]);
        by_parent
            .get(root)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|child| channel_view(store, by_parent, root, &child.channel_h, input, &mut seen))
            .collect()
    } else {
        Vec::new()
    };
    WorkspaceView {
        name: root.to_string(),
        channel: root.to_string(),
        path: store
            .workspace_path(root)
            .ok()
            .flatten()
            .unwrap_or_default(),
        about: meta.map(|channel| channel.about).unwrap_or_default(),
        member_count: members.len(),
        expanded,
        members: if expanded { members } else { Vec::new() },
        channels,
    }
}

fn channel_view(
    store: &Store,
    by_parent: &BTreeMap<String, Vec<Channel>>,
    workspace: &str,
    channel_h: &str,
    input: &AgentWhoInput<'_>,
    seen: &mut BTreeSet<String>,
) -> ChannelView {
    if !seen.insert(channel_h.to_string()) {
        return empty_channel(workspace, channel_h);
    }
    let meta = store.get_channel(channel_h).ok().flatten();
    let name = meta
        .as_ref()
        .and_then(Channel::human_name)
        .unwrap_or(channel_h)
        .to_string();
    let members = member_views(store, channel_h, input);
    let expanded = store
        .is_channel_member(channel_h, input.self_pubkey)
        .unwrap_or(false);
    let children = if expanded {
        by_parent
            .get(channel_h)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|child| channel_view(store, by_parent, workspace, &child.channel_h, input, seen))
            .collect()
    } else {
        Vec::new()
    };
    ChannelView {
        name,
        id: crate::channel_ref::full_channel_ref(store, channel_h),
        about: meta.map(|channel| channel.about).unwrap_or_default(),
        member_count: members.len(),
        expanded,
        members: if expanded { members } else { Vec::new() },
        children,
    }
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

fn member_views(store: &Store, channel: &str, input: &AgentWhoInput<'_>) -> Vec<MemberView> {
    let backend_pubkeys = store
        .list_agent_roster()
        .unwrap_or_default()
        .into_iter()
        .map(|row| row.backend_pubkey)
        .collect::<BTreeSet<_>>();
    let statuses = store
        .live_status_for_channel(channel, input.now)
        .unwrap_or_default()
        .into_iter()
        .map(|status| (status.pubkey.clone(), status))
        .collect::<BTreeMap<_, _>>();
    store
        .list_channel_members(channel)
        .unwrap_or_default()
        .into_iter()
        .filter(|member| {
            member.pubkey != input.backend_pubkey
                && !backend_pubkeys.contains(&member.pubkey)
                && !store
                    .get_profile(&member.pubkey)
                    .ok()
                    .flatten()
                    .is_some_and(|profile| profile.is_backend)
        })
        .map(|member| member_view(store, &member.pubkey, statuses.get(&member.pubkey), input))
        .collect()
}

fn member_view(
    store: &Store,
    pubkey: &str,
    status: Option<&Status>,
    input: &AgentWhoInput<'_>,
) -> MemberView {
    let profile = store.get_profile(pubkey).ok().flatten();
    let is_agent = pubkey == input.self_pubkey
        || status.is_some_and(|row| !row.session_id.is_empty())
        || profile
            .as_ref()
            .is_some_and(|row| !row.agent_slug.is_empty());
    let name = if pubkey == input.self_pubkey {
        input.self_name.to_string()
    } else {
        crate::fabric_context::refs::pubkey_ref(store, pubkey, input.local_host)
    };
    let (state, text, seen) = match status {
        Some(row) => (
            if row.busy { "working" } else { "idle" }.to_string(),
            status_text(row),
            relative_time(row.last_seen, input.now),
        ),
        None => ("offline".to_string(), String::new(), "unknown".to_string()),
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
        seen,
    }
}

fn status_text(status: &Status) -> String {
    let candidates = if status.busy {
        [&status.activity, &status.title]
    } else {
        [&status.title, &status.activity]
    };
    candidates
        .into_iter()
        .find(|text| !text.trim().is_empty())
        .map(|text| text.trim().to_string())
        .unwrap_or_default()
}
