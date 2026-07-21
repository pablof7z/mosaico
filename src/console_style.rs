//! Shared semantic console colors.

use crate::session::Harness;

/// Stable colors for harness-owned surfaces. Unlike workspace colors these are
/// semantic, not hashed: a Claude label should look identical in every picker.
pub(crate) fn harness_color_index(harness: Harness) -> u8 {
    match harness {
        Harness::ClaudeCode => 208,
        Harness::Codex => 45,
        Harness::Opencode => 141,
        Harness::Grok => 75,
        Harness::Goose => 204,
        Harness::Hermes => 214,
        Harness::Unknown => 245,
    }
}

pub(crate) fn harness_ratatui_color(harness: Harness) -> ratatui::style::Color {
    ratatui::style::Color::Indexed(harness_color_index(harness))
}

/// Bright, readable hues that do not overlap busy/success green, warning
/// yellow, or failure red. The workspace root id, not its display name, owns
/// the color so renames do not make a project visually jump.
const WORKSPACE_PALETTE: [u8; 16] = [
    33, 39, 45, 69, 75, 81, 99, 105, 111, 135, 141, 147, 171, 177, 183, 207,
];

pub(crate) fn workspace_color_index(workspace_id: &str) -> u8 {
    let hash = workspace_id
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        });
    WORKSPACE_PALETTE[(hash as usize) % WORKSPACE_PALETTE.len()]
}

pub(crate) fn workspace_ratatui_color(workspace_id: &str) -> ratatui::style::Color {
    ratatui::style::Color::Indexed(workspace_color_index(workspace_id))
}

pub(crate) fn paint_workspace(text: &str, workspace_id: &str, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    format!(
        "\u{1b}[38;5;{}m{text}\u{1b}[0m",
        workspace_color_index(workspace_id)
    )
}

pub(crate) fn paint_stdout_workspace(text: &str, workspace_id: &str) -> String {
    use std::io::IsTerminal as _;
    paint_workspace(text, workspace_id, std::io::stdout().is_terminal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_color_is_stable_and_comes_from_the_semantic_palette() {
        let first = workspace_color_index("root-event-id");
        assert_eq!(first, workspace_color_index("root-event-id"));
        assert!(WORKSPACE_PALETTE.contains(&first));
        assert_ne!(first, workspace_color_index("another-root"));
    }

    #[test]
    fn plain_render_does_not_emit_terminal_escapes() {
        assert_eq!(paint_workspace("mosaico", "root", false), "mosaico");
        assert!(paint_workspace("mosaico", "root", true).contains("\u{1b}[38;5;"));
    }

    #[test]
    fn harness_colors_are_fixed_semantic_colors() {
        assert_eq!(harness_color_index(Harness::Codex), 45);
        assert_eq!(harness_color_index(Harness::Goose), 204);
        assert_ne!(
            harness_color_index(Harness::Codex),
            harness_color_index(Harness::ClaudeCode)
        );
        assert_eq!(harness_color_index(Harness::Hermes), 214);
    }
}
