use super::*;
use crate::fabric_context::workspace_labels::channel_workspace_label;

pub(super) fn render_channel(
    out: &mut String,
    channel: &ChannelBlock,
    color: bool,
    show_workspace: bool,
) {
    let name = format!("#{}", channel.name);
    let workspace = channel_workspace_label(channel, show_workspace)
        .map(|label| format!("  {}", dim(&label, color)))
        .unwrap_or_default();
    if channel.about.is_empty() {
        let _ = writeln!(out, "{}{}", style(&name, color, Style::Channel), workspace);
    } else {
        let _ = writeln!(
            out,
            "{}{}  {}",
            style(&name, color, Style::Channel),
            workspace,
            channel.about
        );
    }
    render_members(out, &channel.members, color);
    render_presence(out, &channel.presence, color);
    render_subchannels(out, &channel.subchannels, color);
    render_messages(out, channel, color);
    out.push('\n');
}
