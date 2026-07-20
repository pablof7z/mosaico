use super::PickerState;
use crate::cli::interactive::session_picker::picker::project::ProjectPicker;
use crate::cli::interactive::session_picker::{HomeChoice, SessionChoice};

impl PickerState {
    pub(in crate::cli::interactive::session_picker::picker) fn refilter(&mut self) {
        let now = crate::util::now_secs();
        let mut scored = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| match choice {
                HomeChoice::Session(choice) => {
                    (!self.query.is_empty() || self.range.includes(&choice.row, now))
                        && self
                            .project_filter
                            .as_deref()
                            .is_none_or(|project| choice.row.belongs_to(project))
                }
                HomeChoice::Agent(_) => true,
            })
            .filter_map(|(index, choice)| {
                choice.fuzzy_score(&self.query).map(|score| (index, score))
            })
            .collect::<Vec<_>>();
        scored.sort_by(|(left_index, left_score), (right_index, right_score)| {
            right_score
                .cmp(left_score)
                .then_with(|| left_index.cmp(right_index))
        });
        self.visible = scored.into_iter().map(|(index, _)| index).collect();
        self.cursor = 0;
        self.offset = 0;
    }

    pub(in crate::cli::interactive::session_picker::picker) fn replace_sessions(
        &mut self,
        sessions: Vec<SessionChoice>,
    ) {
        let selected = self
            .current_choice()
            .map(|index| self.choices[index].stable_id());
        let agents = self
            .choices
            .drain(..)
            .filter(|choice| matches!(choice, HomeChoice::Agent(_)))
            .collect::<Vec<_>>();
        self.choices = sessions
            .into_iter()
            .map(HomeChoice::Session)
            .chain(agents)
            .collect();
        self.refilter();
        if let Some(selected) = selected {
            self.cursor = self
                .visible
                .iter()
                .position(|&index| self.choices[index].stable_id() == selected)
                .unwrap_or(0);
        }
        self.refresh_project_picker();
    }

    fn refresh_project_picker(&mut self) {
        let Some(projects) = self.project_picker.take() else {
            return;
        };
        let focused = projects
            .visible
            .get(projects.cursor)
            .and_then(|&index| projects.options[index].id.clone());
        let mut refreshed = ProjectPicker::new(&self.choices, self.project_filter.as_deref());
        refreshed.query = projects.query;
        refreshed.refilter();
        if let Some(focused) = focused {
            refreshed.cursor = refreshed
                .visible
                .iter()
                .position(|&index| refreshed.options[index].id.as_deref() == Some(&focused))
                .unwrap_or(0);
        }
        self.project_picker = Some(refreshed);
    }

    pub(in crate::cli::interactive::session_picker::picker) fn project_label(&self) -> &str {
        let Some(id) = self.project_filter.as_deref() else {
            return "All";
        };
        self.choices
            .iter()
            .filter_map(|choice| match choice {
                HomeChoice::Session(choice) => Some(&choice.row),
                HomeChoice::Agent(_) => None,
            })
            .flat_map(|row| row.workspaces.iter())
            .find(|workspace| workspace.id == id)
            .map(|workspace| workspace.name.as_str())
            .unwrap_or(id)
    }
}
