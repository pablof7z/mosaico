use super::*;

pub(super) fn render_channel(out: &mut String, channel: &ChannelBlock, color: bool, indent: usize) {
    let pad = " ".repeat(indent);
    let name = channel.path.clone();
    if channel.about.is_empty() {
        let _ = writeln!(out, "{pad}{}", style(&name, color, Style::Channel));
    } else {
        let _ = writeln!(
            out,
            "{pad}{}  {}",
            style(&name, color, Style::Channel),
            channel.about
        );
    }
    render_channel_body(out, channel, color);
    for child in &channel.children {
        render_channel(out, child, color, indent + 2);
    }
}
