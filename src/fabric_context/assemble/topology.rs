use std::collections::{BTreeMap, BTreeSet};

use super::{members, message_rows, presence_rows};
use crate::fabric_context::capture::{ChannelCap, ViewInputs, WorkspaceCap};
use crate::fabric_context::model::{ChannelBlock, WorkspaceView};
use crate::util::relative_time;

pub(super) fn workspace_rows(
    inputs: &ViewInputs,
    cursor: u64,
    now: u64,
    full: bool,
) -> Option<Vec<WorkspaceView>> {
    let rows = inputs
        .meta
        .workspaces
        .iter()
        .filter_map(|workspace| workspace_row(inputs, workspace, cursor, now, full))
        .collect::<Vec<_>>();
    (full || !rows.is_empty()).then_some(rows)
}

fn workspace_row(
    inputs: &ViewInputs,
    workspace: &WorkspaceCap,
    cursor: u64,
    now: u64,
    full: bool,
) -> Option<WorkspaceView> {
    let caps = workspace
        .channels
        .iter()
        .map(|channel| (channel.reference.clone(), channel))
        .collect::<BTreeMap<_, _>>();
    let expanded = inputs.meta.self_row.is_none() || workspace_is_expanded(inputs, workspace);
    let mut selected = if full {
        full_channel_ids(inputs, workspace, &caps)
    } else if expanded {
        delta_channel_ids(inputs, workspace, cursor, now)
    } else {
        compact_delta_ids(workspace, &caps, cursor, now)
    };
    let workspace_changed = !full && workspace.updated_at > cursor && workspace.updated_at <= now;
    if !expanded {
        selected.retain(|id| id == &workspace.summary.channel);
        if full || workspace_changed {
            selected.insert(workspace.summary.channel.clone());
        }
    }
    if !full && selected.is_empty() && !workspace_changed {
        return None;
    }
    let content = if expanded {
        selected.clone()
    } else {
        BTreeSet::new()
    };
    let selected = if full {
        selected
    } else {
        with_ancestors(&selected, &caps)
    };
    let blocks = selected
        .iter()
        .filter_map(|id| {
            caps.get(id).map(|channel| {
                channel_block(inputs, channel, content.contains(id), full, cursor, now)
            })
        })
        .collect();
    let (mut root, channels) =
        crate::fabric_context::tree::arrange(&workspace.summary.name, blocks);
    if root.is_none() && selected.contains(&workspace.summary.channel) {
        root = Some(compact_root(inputs, workspace));
    }
    Some(WorkspaceView {
        name: workspace.summary.name.clone(),
        about: workspace.summary.about.clone(),
        hosts: workspace.hosts.clone(),
        root,
        channels,
    })
}

fn compact_delta_ids(
    workspace: &WorkspaceCap,
    caps: &BTreeMap<String, &ChannelCap>,
    cursor: u64,
    now: u64,
) -> BTreeSet<String> {
    caps.get(&workspace.summary.channel)
        .filter(|root| root.updated_at > cursor && root.updated_at <= now)
        .map(|_| BTreeSet::from([workspace.summary.channel.clone()]))
        .unwrap_or_default()
}

fn compact_root(inputs: &ViewInputs, workspace: &WorkspaceCap) -> ChannelBlock {
    ChannelBlock {
        path: workspace.summary.channel.clone(),
        about: workspace.summary.about.clone(),
        agent_count: named_agent_count(inputs, &workspace.summary.name),
        last_active: None,
        members: Vec::new(),
        presence: Vec::new(),
        departures: Vec::new(),
        children: Vec::new(),
        messages: Vec::new(),
        omitted: 0,
    }
}

fn full_channel_ids(
    inputs: &ViewInputs,
    workspace: &WorkspaceCap,
    caps: &BTreeMap<String, &ChannelCap>,
) -> BTreeSet<String> {
    if inputs.meta.self_row.is_none() {
        return caps.keys().cloned().collect();
    }
    let root = workspace.summary.channel.clone();
    let mut selected = BTreeSet::new();
    if caps.contains_key(&root) {
        selected.insert(root.clone());
    }
    if workspace_is_expanded(inputs, workspace) {
        add_visible_children(&root, caps, &mut selected);
    }
    selected
}

fn workspace_is_expanded(inputs: &ViewInputs, workspace: &WorkspaceCap) -> bool {
    workspace
        .channels
        .iter()
        .any(|channel| inputs.meta.joined_channels.contains(&channel.h))
}

fn add_visible_children(
    parent: &str,
    caps: &BTreeMap<String, &ChannelCap>,
    selected: &mut BTreeSet<String>,
) {
    for (id, _) in caps
        .iter()
        .filter(|(id, _)| parent_id(id).is_some_and(|candidate| candidate == parent))
    {
        selected.insert(id.clone());
        add_visible_children(id, caps, selected);
    }
}

fn delta_channel_ids(
    inputs: &ViewInputs,
    workspace: &WorkspaceCap,
    cursor: u64,
    now: u64,
) -> BTreeSet<String> {
    workspace
        .channels
        .iter()
        .filter(|channel| {
            (channel.updated_at > cursor && channel.updated_at <= now)
                || ((inputs.meta.self_row.is_none() || is_member(inputs, &channel.h))
                    && !presence_rows(inputs, &channel.h, cursor, now).is_empty())
                || inputs
                    .messages
                    .channels
                    .get(&channel.h)
                    .is_some_and(|bundle| !message_rows(bundle, cursor, now).0.is_empty())
        })
        .map(|channel| channel.reference.clone())
        .collect()
}

fn with_ancestors(
    content: &BTreeSet<String>,
    caps: &BTreeMap<String, &ChannelCap>,
) -> BTreeSet<String> {
    let mut selected = content.clone();
    for id in content {
        let mut current = id.as_str();
        while let Some(parent) = parent_id(current) {
            if !caps.contains_key(parent) {
                break;
            }
            selected.insert(parent.to_string());
            current = parent;
        }
    }
    selected
}

fn channel_block(
    inputs: &ViewInputs,
    channel: &ChannelCap,
    content: bool,
    full: bool,
    cursor: u64,
    now: u64,
) -> ChannelBlock {
    let member = is_member(inputs, &channel.h);
    let agent_count = named_agent_count(inputs, &channel.h);
    let last_active = channel.latest_message_at.map(|at| relative_time(at, now));
    let members = if content && full && (member || inputs.meta.self_row.is_none()) {
        members::member_rows(inputs, &channel.h, now)
    } else {
        Vec::new()
    };
    let presence = if content && (member || inputs.meta.self_row.is_none()) {
        presence_rows(inputs, &channel.h, cursor, now)
    } else {
        Vec::new()
    };
    let (messages, omitted) = if content {
        inputs
            .messages
            .channels
            .get(&channel.h)
            .map(|bundle| message_rows(bundle, cursor, now))
            .unwrap_or_default()
    } else {
        Default::default()
    };
    ChannelBlock {
        path: channel.reference.clone(),
        about: channel.about.clone(),
        agent_count,
        last_active,
        members,
        presence,
        departures: Vec::new(),
        children: Vec::new(),
        messages,
        omitted,
    }
}

fn is_member(inputs: &ViewInputs, channel: &str) -> bool {
    !inputs.meta.self_pubkey.is_empty()
        && inputs.meta.joined_channels.contains(channel)
        && inputs.members.hydrated.contains(channel)
        && inputs
            .members
            .roster
            .get(channel)
            .is_some_and(|members| members.contains_key(&inputs.meta.self_pubkey))
}

fn named_agent_count(inputs: &ViewInputs, channel: &str) -> Option<usize> {
    if !inputs.members.hydrated.contains(channel) {
        return None;
    }
    let statuses = inputs
        .presence
        .statuses
        .get(channel)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut count = 0;
    for (pubkey, role) in inputs
        .members
        .roster
        .get(channel)
        .into_iter()
        .flat_map(|members| members.iter())
    {
        let is_named_agent = pubkey.as_str() == inputs.meta.self_pubkey
            || inputs
                .members
                .agent_slugs
                .get(pubkey.as_str())
                .is_some_and(|slug| !slug.trim().is_empty())
            || statuses
                .iter()
                .any(|status| status.pubkey == pubkey.as_str() && !status.slug.trim().is_empty());
        match crate::agent_count::classify(
            role,
            inputs.members.backend.contains(pubkey.as_str()),
            inputs.members.known_profiles.contains(pubkey.as_str()),
            is_named_agent,
        ) {
            crate::agent_count::MemberClass::Agent => count += 1,
            crate::agent_count::MemberClass::Unknown => return None,
            crate::agent_count::MemberClass::Ignore | crate::agent_count::MemberClass::Human => {}
        }
    }
    Some(count)
}

fn parent_id(id: &str) -> Option<&str> {
    id.rsplit_once('/')
        .map(|(parent, _)| parent)
        .filter(|parent| !parent.is_empty())
}
