use std::collections::{BTreeMap, BTreeSet, HashSet};

use anyhow::Result;

use super::{ChannelList, ChannelListEntry, ChannelListSection};
use crate::state::{Channel, Store};

mod member_facts;

pub(super) enum ListMode {
    Caller {
        own: String,
        joined: BTreeSet<String>,
    },
    All,
    Recursive,
    Workspace(String),
}

struct Inputs {
    roots: BTreeSet<String>,
    nodes: BTreeMap<String, Node>,
    children: BTreeMap<String, Vec<String>>,
}

#[derive(Clone)]
struct Node {
    path: String,
    about: String,
    agents: Option<usize>,
    last_activity: Option<String>,
}

pub(super) fn build(
    store: &Store,
    mode: ListMode,
    now: u64,
    local_backend: &str,
) -> Result<ChannelList> {
    let inputs = capture(store, now, local_backend, roots_required_by(&mode))?;
    let sections = match mode {
        ListMode::Caller { own, joined } => caller_sections(&inputs, &own, &joined),
        ListMode::All => vec![section(
            "all",
            "All workspaces",
            compact_roots(&inputs, inputs.roots.iter()),
        )],
        ListMode::Recursive => vec![section(
            "all",
            "All workspaces",
            expanded_roots(&inputs, inputs.roots.iter()),
        )],
        ListMode::Workspace(root) => vec![section(
            "workspace",
            "Workspace",
            expanded_roots(&inputs, std::iter::once(&root)),
        )],
    };
    Ok(ChannelList { sections })
}

fn roots_required_by(mode: &ListMode) -> impl Iterator<Item = &str> {
    let mut roots = Vec::new();
    match mode {
        ListMode::Caller { own, joined } => {
            roots.extend((!own.is_empty()).then_some(own.as_str()));
            roots.extend(joined.iter().map(String::as_str));
        }
        ListMode::Workspace(root) => roots.push(root.as_str()),
        ListMode::All | ListMode::Recursive => {}
    }
    roots.into_iter()
}

fn capture<'a>(
    store: &Store,
    now: u64,
    local_backend: &str,
    required_roots: impl Iterator<Item = &'a str>,
) -> Result<Inputs> {
    let all_channels = store.list_channels()?;
    let archived_roots = all_channels
        .iter()
        .filter(|channel| channel.parent.is_empty() && channel.is_archived())
        .map(|channel| channel.channel_h.clone())
        .collect::<BTreeSet<_>>();
    let channels = all_channels
        .into_iter()
        .filter(|channel| !channel.is_archived())
        .collect::<Vec<_>>();
    let activity = store.latest_accepted_message_at_by_channel()?;
    let member_index = crate::agent_count::MemberFactIndex::capture(store, local_backend)?;
    let mut roots = channels
        .iter()
        .filter(|channel| channel.parent.is_empty())
        .map(|channel| channel.channel_h.clone())
        .collect::<BTreeSet<_>>();
    roots.extend(
        store
            .list_workspace_bindings()?
            .into_iter()
            .map(|binding| binding.channel_h)
            .filter(|root| !archived_roots.contains(root)),
    );
    roots.extend(
        required_roots
            .filter(|root| !root.is_empty())
            .map(str::to_string),
    );

    let mut nodes = BTreeMap::new();
    let mut children = BTreeMap::<String, Vec<String>>::new();
    for channel in channels {
        let path = crate::channel_ref::full_channel_ref(store, &channel.channel_h);
        if path.is_empty() {
            continue;
        }
        if !channel.parent.is_empty() {
            children
                .entry(channel.parent.clone())
                .or_default()
                .push(channel.channel_h.clone());
        }
        nodes.insert(
            channel.channel_h.clone(),
            node(
                store,
                &channel,
                path,
                activity.get(&channel.channel_h),
                now,
                &member_index,
            )?,
        );
    }
    for children in children.values_mut() {
        children.sort_by_key(|channel| nodes.get(channel).map(|node| node.path.clone()));
    }
    for root in &roots {
        if !nodes.contains_key(root) {
            let (hydrated, members) = member_facts::capture(store, root, &member_index)?;
            nodes.insert(
                root.clone(),
                Node {
                    path: crate::channel_ref::format_channel_ref(root, &[]),
                    about: String::new(),
                    agents: crate::agent_count::count_agents(hydrated, members),
                    last_activity: activity
                        .get(root)
                        .map(|at| crate::util::relative_time(*at, now)),
                },
            );
        }
    }
    Ok(Inputs {
        roots,
        nodes,
        children,
    })
}

fn node(
    store: &Store,
    channel: &Channel,
    path: String,
    activity: Option<&u64>,
    now: u64,
    member_index: &crate::agent_count::MemberFactIndex,
) -> Result<Node> {
    let (hydrated, members) = member_facts::capture(store, &channel.channel_h, member_index)?;
    Ok(Node {
        path,
        about: channel.about.clone(),
        agents: crate::agent_count::count_agents(hydrated, members),
        last_activity: activity.map(|at| crate::util::relative_time(*at, now)),
    })
}

fn caller_sections(
    inputs: &Inputs,
    own: &str,
    joined: &BTreeSet<String>,
) -> Vec<ChannelListSection> {
    let own_roots = inputs.roots.iter().filter(|root| root.as_str() == own);
    let own_entries = if joined.contains(own) {
        expanded_roots(inputs, own_roots)
    } else {
        compact_roots(inputs, own_roots)
    };
    let joined_roots = inputs
        .roots
        .iter()
        .filter(|root| root.as_str() != own && joined.contains(*root));
    let other_roots = inputs
        .roots
        .iter()
        .filter(|root| root.as_str() != own && !joined.contains(*root));
    vec![
        section("own", "Your workspace", own_entries),
        section(
            "joined",
            "Joined workspaces",
            expanded_roots(inputs, joined_roots),
        ),
        section(
            "other",
            "Other workspaces",
            compact_roots(inputs, other_roots),
        ),
    ]
}

fn expanded_roots<'a>(
    inputs: &Inputs,
    roots: impl Iterator<Item = &'a String>,
) -> Vec<ChannelListEntry> {
    roots
        .filter_map(|root| expanded_entry(inputs, root, &mut HashSet::new()))
        .collect()
}

fn expanded_entry(
    inputs: &Inputs,
    channel: &str,
    seen: &mut HashSet<String>,
) -> Option<ChannelListEntry> {
    if !seen.insert(channel.to_string()) {
        return None;
    }
    let node = inputs.nodes.get(channel)?;
    let children = inputs
        .children
        .get(channel)
        .into_iter()
        .flatten()
        .filter_map(|child| expanded_entry(inputs, child, seen))
        .collect();
    Some(entry(node, None, children))
}

fn compact_roots<'a>(
    inputs: &Inputs,
    roots: impl Iterator<Item = &'a String>,
) -> Vec<ChannelListEntry> {
    roots
        .filter_map(|root| {
            let node = inputs.nodes.get(root)?;
            let count = descendant_count(inputs, root, &mut HashSet::new());
            Some(entry(node, (count > 0).then_some(count), Vec::new()))
        })
        .collect()
}

fn descendant_count(inputs: &Inputs, channel: &str, seen: &mut HashSet<String>) -> usize {
    if !seen.insert(channel.to_string()) {
        return 0;
    }
    inputs
        .children
        .get(channel)
        .into_iter()
        .flatten()
        .filter(|child| inputs.nodes.contains_key(*child))
        .map(|child| 1 + descendant_count(inputs, child, seen))
        .sum()
}

fn entry(
    node: &Node,
    subchannels: Option<usize>,
    children: Vec<ChannelListEntry>,
) -> ChannelListEntry {
    ChannelListEntry {
        path: node.path.clone(),
        about: node.about.clone(),
        agents: node.agents,
        last_activity: node.last_activity.clone(),
        subchannels,
        children,
    }
}

fn section(
    kind: &'static str,
    title: &'static str,
    channels: Vec<ChannelListEntry>,
) -> ChannelListSection {
    ChannelListSection {
        kind,
        title,
        channels,
    }
}
