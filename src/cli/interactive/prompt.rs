//! Shared prompt chrome and cancellation semantics for inline `inquire` flows.

use anyhow::Result;
use inquire::ui::{Color, ErrorMessageRenderConfig, RenderConfig, StyleSheet, Styled};
use inquire::InquireError;

const ACCENT: Color = Color::AnsiValue(45);
const SUCCESS: Color = Color::AnsiValue(78);
const ERROR: Color = Color::AnsiValue(203);
const MUTED: Color = Color::AnsiValue(245);

pub(in crate::cli) fn install_theme() {
    inquire::set_global_render_config(theme());
}

/// Convert cancel/interrupt into `None`, so inline prompts exit without an
/// error after Esc or Ctrl-C.
pub(in crate::cli) fn prompted<T>(r: std::result::Result<T, InquireError>) -> Result<Option<T>> {
    match r {
        Ok(value) => Ok(Some(value)),
        Err(InquireError::OperationCanceled) | Err(InquireError::OperationInterrupted) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn theme() -> RenderConfig<'static> {
    let mut config = RenderConfig::default_colored()
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
    config.placeholder = StyleSheet::new().with_fg(MUTED);
    config
}
