//! Semantic palette and accessibility gates for the onboarding TUI.
//!
//! Color is used semantically, never as decoration, and every color decision
//! collapses to the terminal default when `NO_COLOR` is set. Motion collapses
//! to a static frame under `NO_COLOR` or `MOSAICO_NO_ANIM`.

use ratatui::style::{Color, Modifier, Style};

pub(super) const ACCENT: Color = Color::Indexed(45); // bright cyan — brand
pub(super) const ACCENT_ALT: Color = Color::Indexed(213); // pink — secondary brand
pub(super) const MUTED: Color = Color::Indexed(245);
pub(super) const FAINT: Color = Color::Indexed(240);
pub(super) const OK: Color = Color::Indexed(42);
pub(super) const WARN: Color = Color::Indexed(214);
pub(super) const ERR: Color = Color::Indexed(203);

/// Whether ANSI color may be emitted at all.
pub(super) fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// Whether animation should collapse to a single static frame.
pub(super) fn reduced_motion() -> bool {
    std::env::var_os("NO_COLOR").is_some() || std::env::var_os("MOSAICO_NO_ANIM").is_some()
}

/// A foreground style that becomes the terminal default under `NO_COLOR`.
pub(super) fn fg(color: Color) -> Style {
    if color_enabled() {
        Style::default().fg(color)
    } else {
        Style::default()
    }
}

/// A foreground style with a modifier, color-gated. The modifier (e.g. bold,
/// underline) survives `NO_COLOR` so state stays legible without color.
pub(super) fn fg_mod(color: Color, modifier: Modifier) -> Style {
    fg(color).add_modifier(modifier)
}

/// A bold, color-gated style; bold survives `NO_COLOR` as a non-color signal.
pub(super) fn bold(color: Color) -> Style {
    fg_mod(color, Modifier::BOLD)
}
