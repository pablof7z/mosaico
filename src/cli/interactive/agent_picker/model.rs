use crate::session::Harness;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) struct AgentProvenance {
    pub(in crate::cli) label: String,
    pub(in crate::cli) harness: Harness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::cli) struct AgentPickerRow {
    pub(in crate::cli) name: String,
    pub(in crate::cli) description: String,
    pub(in crate::cli) status: Option<AgentProvenance>,
}

impl AgentPickerRow {
    pub(super) fn fuzzy_score(&self, input: &str) -> Option<i64> {
        if input.is_empty() {
            return Some(0);
        }
        let matcher = SkimMatcherV2::default().ignore_case();
        [
            (self.name.as_str(), 4_000),
            (self.description.as_str(), 2_000),
            (
                self.status
                    .as_ref()
                    .map(|value| value.label.as_str())
                    .unwrap_or_default(),
                750,
            ),
        ]
        .into_iter()
        .filter_map(|(field, priority)| {
            let score = matcher.fuzzy_match(field, input)?;
            let exact = i64::from(field.to_lowercase().contains(&input.to_lowercase())) * 10_000;
            Some(score + exact + priority)
        })
        .max()
    }
}
