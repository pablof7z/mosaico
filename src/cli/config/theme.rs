//! Color palette and prompt chrome for `tenex-edge config`'s interactive
//! flows: one accent color, heavy dimming for secondary text, no borders —
//! matching the existing ratatui session picker. These ANSI-256 values are
//! chosen to hold ≥3:1 contrast on both a near-black and a near-white
//! background, since we can't detect the user's terminal theme.
//!
//! `NO_COLOR` (checked by `owo-colors`/`inquire` themselves) disables color;
//! `pub(super) const` values below only ever set *foreground*, never a
//! background fill, so there's nothing left to clash once color is off.

use inquire::ui::{Color, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled};

const ACCENT: Color = Color::AnsiValue(45);
const SUCCESS: Color = Color::AnsiValue(78);
const ERROR: Color = Color::AnsiValue(203);
const MUTED: Color = Color::AnsiValue(245);

/// Install this tool's palette as the global `inquire` render config. Call
/// once before any prompt is shown.
pub(super) fn install() {
    inquire::set_global_render_config(theme());
}

fn theme() -> RenderConfig<'static> {
    let mut cfg = RenderConfig::default_colored()
        .with_prompt_prefix(Styled::new("?").with_fg(ACCENT))
        .with_answered_prompt_prefix(Styled::new("✓").with_fg(SUCCESS))
        .with_highlighted_option_prefix(Styled::new("❯").with_fg(ACCENT))
        .with_selected_option(Some(StyleSheet::new().with_fg(ACCENT)))
        .with_help_message(StyleSheet::new().with_fg(MUTED))
        .with_answer(StyleSheet::new().with_fg(ACCENT))
        .with_canceled_prompt_indicator(Styled::new("(cancelled)").with_fg(MUTED))
        .with_error_message(
            ErrorMessageRenderConfig::default_colored()
                .with_prefix(Styled::new("✗").with_fg(ERROR)),
        );
    cfg.placeholder = StyleSheet::new().with_fg(MUTED);
    cfg
}
