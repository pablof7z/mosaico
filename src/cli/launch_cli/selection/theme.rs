use console::Style;
use dialoguer::theme::{ColorfulTheme, Theme};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use std::fmt;

pub(super) const ROW_SEPARATOR: char = '\u{1f}';

#[derive(Default)]
pub(super) struct LaunchTheme {
    base: ColorfulTheme,
}

impl Theme for LaunchTheme {
    fn format_fuzzy_select_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        search_term: &str,
        bytes_pos: usize,
    ) -> fmt::Result {
        self.base
            .format_fuzzy_select_prompt(f, prompt, search_term, bytes_pos)
    }

    fn format_input_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: &str,
    ) -> fmt::Result {
        self.base
            .format_input_prompt_selection(f, prompt, &selection.replace(ROW_SEPARATOR, "  "))
    }

    fn format_fuzzy_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        active: bool,
        highlight_matches: bool,
        matcher: &SkimMatcherV2,
        search_term: &str,
    ) -> fmt::Result {
        let prefix = if active {
            Style::new().for_stderr().green().bold().apply_to("❯")
        } else {
            Style::new().for_stderr().apply_to(" ")
        };
        write!(f, "{prefix} ")?;
        let matches = highlight_matches
            .then(|| matcher.fuzzy_indices(text, search_term))
            .flatten()
            .map(|(_, indices)| indices)
            .unwrap_or_default();
        let mut in_name = true;
        for (index, character) in text.chars().enumerate() {
            if character == ROW_SEPARATOR {
                write!(f, "  ")?;
                in_name = false;
                continue;
            }
            let style = if matches.contains(&index) {
                Style::new().for_stderr().yellow().bold()
            } else if in_name && active {
                Style::new().for_stderr().cyan().bold()
            } else if in_name {
                Style::new().for_stderr().white().bold()
            } else {
                Style::new().for_stderr().black().bright()
            };
            write!(f, "{}", style.apply_to(character))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_separator_is_replaced_in_reported_selection() {
        let mut rendered = String::new();
        LaunchTheme::default()
            .format_input_prompt_selection(&mut rendered, "Launch", "codex\u{1f}Codex harness")
            .unwrap();

        assert!(!rendered.contains(ROW_SEPARATOR));
        assert!(rendered.contains("codex  Codex harness"));
    }
}
