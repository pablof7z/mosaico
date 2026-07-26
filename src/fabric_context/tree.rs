use super::model::ChannelBlock;
use std::collections::BTreeMap;

/// Merge detailed joined-channel rows with compact descendant rows, then attach
/// every node beneath the parent named by its canonical slash reference.
pub(super) fn arrange(
    workspace: &str,
    channels: Vec<ChannelBlock>,
) -> (Option<ChannelBlock>, Vec<ChannelBlock>) {
    let mut nodes = BTreeMap::new();
    for channel in channels {
        collect(&mut nodes, channel);
    }

    let mut references = nodes.keys().cloned().collect::<Vec<_>>();
    references.sort_by_key(|reference| std::cmp::Reverse(depth(reference)));
    let root_reference = crate::channel_ref::format_channel_ref(workspace, &[]);
    for reference in references {
        if reference == root_reference {
            continue;
        }
        let Some((parent, _)) = reference.rsplit_once('/') else {
            continue;
        };
        if !nodes.contains_key(parent) {
            continue;
        }
        let child = nodes
            .remove(&reference)
            .expect("tree reference came from the node map");
        nodes
            .get_mut(parent)
            .expect("parent existence checked above")
            .children
            .push(child);
    }

    let mut root = nodes.remove(&root_reference);
    if let Some(root) = &mut root {
        sort_children(root);
    }
    let mut top = nodes.into_values().collect::<Vec<_>>();
    top.sort_by(|a, b| a.path.cmp(&b.path));
    for channel in &mut top {
        sort_children(channel);
    }
    (root, top)
}

fn collect(nodes: &mut BTreeMap<String, ChannelBlock>, mut channel: ChannelBlock) {
    let children = std::mem::take(&mut channel.children);
    let reference = channel.path.clone();
    if let Some(existing) = nodes.get_mut(&reference) {
        merge(existing, channel);
    } else {
        nodes.insert(reference, channel);
    }
    for child in children {
        collect(nodes, child);
    }
}

fn merge(existing: &mut ChannelBlock, incoming: ChannelBlock) {
    if !incoming.about.is_empty() {
        existing.about = incoming.about;
    }
    if incoming.agent_count.is_some() {
        existing.agent_count = incoming.agent_count;
    }
    if incoming.last_active.is_some() {
        existing.last_active = incoming.last_active;
    }
    if !incoming.members.is_empty() {
        existing.members = incoming.members;
    }
    if !incoming.presence.is_empty() {
        existing.presence = incoming.presence;
    }
    if !incoming.messages.is_empty() {
        existing.messages = incoming.messages;
    }
    existing.omitted = existing.omitted.max(incoming.omitted);
}

fn sort_children(channel: &mut ChannelBlock) {
    channel.children.sort_by(|a, b| a.path.cmp(&b.path));
    for child in &mut channel.children {
        sort_children(child);
    }
}

fn depth(reference: &str) -> usize {
    reference.bytes().filter(|byte| *byte == b'/').count()
}
