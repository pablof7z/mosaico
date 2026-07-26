//! Narrow tree-mutation helpers for semantic member deltas.

use super::super::model::{ChannelBlock, FabricView, MemberRow, PresenceRow, WorkspaceView};

pub(super) fn add_presence(view: &mut FabricView, reference: &str, row: PresenceRow) {
    let channel = channel_mut(view, reference);
    if channel
        .presence
        .iter()
        .any(|current| current.name == row.name)
    {
        return;
    }
    channel.members.retain(|current| current.name != row.name);
    channel.presence.push(row);
}

pub(super) fn add_member(view: &mut FabricView, reference: &str, row: MemberRow) {
    let channel = channel_mut(view, reference);
    if channel
        .presence
        .iter()
        .any(|current| current.name == row.name)
        || channel
            .members
            .iter()
            .any(|current| current.name == row.name)
    {
        return;
    }
    channel.members.push(row);
}

pub(super) fn channel_mut<'a>(view: &'a mut FabricView, reference: &str) -> &'a mut ChannelBlock {
    if let Some(index) = find_channel_index(view, reference) {
        return channel_at_index(view, &index);
    }
    let block = empty_channel(reference);
    let workspaces = view.workspaces.get_or_insert_with(Vec::new);
    workspaces.push(WorkspaceView {
        name: String::new(),
        about: String::new(),
        hosts: Vec::new(),
        root: None,
        channels: vec![block],
    });
    workspaces
        .last_mut()
        .and_then(|workspace| workspace.channels.last_mut())
        .expect("the channel was just inserted")
}

fn find_channel_index(view: &FabricView, reference: &str) -> Option<Vec<usize>> {
    for (workspace_index, workspace) in view.workspaces.iter().flatten().enumerate() {
        if let Some(root) = &workspace.root {
            if let Some(mut child_path) = find_child_index(root, reference) {
                child_path.insert(0, usize::MAX);
                child_path.insert(0, workspace_index);
                return Some(child_path);
            }
        }
        for (channel_index, channel) in workspace.channels.iter().enumerate() {
            if let Some(mut child_path) = find_child_index(channel, reference) {
                child_path.insert(0, channel_index);
                child_path.insert(0, workspace_index);
                return Some(child_path);
            }
        }
    }
    None
}

fn find_child_index(channel: &ChannelBlock, reference: &str) -> Option<Vec<usize>> {
    if channel.path == reference {
        return Some(Vec::new());
    }
    for (index, child) in channel.children.iter().enumerate() {
        if let Some(mut path) = find_child_index(child, reference) {
            path.insert(0, index);
            return Some(path);
        }
    }
    None
}

fn channel_at_index<'a>(view: &'a mut FabricView, path: &[usize]) -> &'a mut ChannelBlock {
    let workspace = &mut view.workspaces.as_mut().expect("located workspace")[path[0]];
    let mut channel = if path[1] == usize::MAX {
        workspace.root.as_mut().expect("located root")
    } else {
        &mut workspace.channels[path[1]]
    };
    for index in &path[2..] {
        channel = &mut channel.children[*index];
    }
    channel
}

fn empty_channel(reference: &str) -> ChannelBlock {
    ChannelBlock {
        path: reference.to_string(),
        about: String::new(),
        agent_count: None,
        last_active: None,
        members: Vec::new(),
        presence: Vec::new(),
        departures: Vec::new(),
        children: Vec::new(),
        messages: Vec::new(),
        omitted: 0,
    }
}
