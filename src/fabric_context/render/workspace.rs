use std::fmt::Write as _;

use super::{esc_attr, render_channel, render_channel_body};
use crate::fabric_context::model::{ChannelBlock, WorkspaceRow};

pub(super) fn render_workspace_block(
    out: &mut String,
    workspace: &WorkspaceRow,
    root: Option<&ChannelBlock>,
    channels: &[ChannelBlock],
) {
    let _ = write!(
        out,
        "\n\n  <workspace name=\"{}\"",
        esc_attr(&workspace.name)
    );
    if !workspace.channel.is_empty() {
        let _ = write!(out, " channel=\"{}\"", esc_attr(&workspace.channel));
    }
    if !workspace.about.is_empty() {
        let _ = write!(out, " about=\"{}\"", esc_attr(&workspace.about));
    }
    out.push('>');
    if let Some(root) = root {
        render_channel_body(out, root, 4);
    }
    if root.is_some_and(|root| !root.children.is_empty()) || !channels.is_empty() {
        out.push_str("\n    <channels>");
        if let Some(root) = root {
            for channel in &root.children {
                render_channel(out, channel, 6);
            }
        }
        for channel in channels {
            render_channel(out, channel, 6);
        }
        out.push_str("\n    </channels>");
    }
    out.push_str("\n  </workspace>");
}
