use super::model::ChannelBlock;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::fabric_context) fn channels_need_workspace(
    channels: &[ChannelBlock],
    current_workspace: &str,
) -> bool {
    let mut by_name: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for channel in channels {
        let workspace = channel.workspace.trim();
        if !workspace.is_empty() && workspace != current_workspace {
            return true;
        }
        by_name
            .entry(channel.name.as_str())
            .or_default()
            .insert(workspace);
    }
    by_name.values().any(|workspaces| workspaces.len() > 1)
}

pub(in crate::fabric_context) fn channel_workspace(
    channel: &ChannelBlock,
    show_workspace: bool,
) -> Option<&str> {
    let workspace = channel.workspace.trim();
    (show_workspace && !workspace.is_empty()).then_some(workspace)
}

pub(in crate::fabric_context) fn channel_workspace_label(
    channel: &ChannelBlock,
    show_workspace: bool,
) -> Option<String> {
    channel_workspace(channel, show_workspace).map(|workspace| format!("workspace {workspace}"))
}
