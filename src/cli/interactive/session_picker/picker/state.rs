use super::{confirmation, project::ProjectPicker, PickerExit, SessionScope};
use crate::cli::interactive::session_picker::SessionChoice;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug)]
pub(super) struct PickerState {
    pub(super) choices: Vec<SessionChoice>,
    pub(super) visible: Vec<usize>,
    pub(super) query: String,
    pub(super) scope: SessionScope,
    pub(super) project_filter: Option<String>,
    pub(super) project_picker: Option<ProjectPicker>,
    pub(super) notice: Option<String>,
    pub(super) confirmation: Option<confirmation::Confirmation>,
    pub(super) cursor: usize,
    pub(super) offset: usize,
}

impl PickerState {
    pub(super) fn new(choices: Vec<SessionChoice>) -> Self {
        let mut state = Self {
            choices,
            visible: Vec::new(),
            query: String::new(),
            scope: SessionScope::Live,
            project_filter: None,
            project_picker: None,
            notice: None,
            confirmation: None,
            cursor: 0,
            offset: 0,
        };
        state.refilter();
        state
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, rows: usize) -> Option<PickerExit> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if self.project_picker.is_some() {
            self.handle_project_key(key, rows.saturating_mul(2));
            return None;
        }
        if self.confirmation.is_some() {
            return self.handle_confirmation(key);
        }
        self.notice = None;
        match key.code {
            KeyCode::Esc => return Some(PickerExit::Cancel),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PickerExit::Cancel);
            }
            KeyCode::Char('K') | KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                return self.current_choice().map(PickerExit::Kill);
            }
            KeyCode::Char('p') if self.query.is_empty() && key.modifiers.is_empty() => {
                self.project_picker = Some(ProjectPicker::new(
                    &self.choices,
                    self.project_filter.as_deref(),
                ));
            }
            KeyCode::Tab => {
                self.scope.toggle();
                self.refilter();
            }
            KeyCode::Enter => {
                let choice = self.current_choice()?;
                let row = &self.choices[choice].row;
                if row.attachable() {
                    return Some(PickerExit::Attach(choice));
                }
                if row.resumable && !row.running {
                    return Some(PickerExit::Resume(choice));
                }
                if row.can_take_over() {
                    self.confirmation = Some(confirmation::Confirmation::TakeOver(choice));
                    return None;
                }
                self.notice = Some(if !row.running {
                    "This exited session has no restartable harness state".into()
                } else if row.transport == "acp" {
                    "ACP sessions run without a harness terminal — nothing to attach to".into()
                } else {
                    "This session has no live attachable terminal".into()
                });
            }
            KeyCode::Up => self.move_up(1),
            KeyCode::Down => self.move_down(1),
            KeyCode::PageUp => self.move_up(rows.max(1)),
            KeyCode::PageDown => self.move_down(rows.max(1)),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.visible.len().saturating_sub(1),
            KeyCode::Backspace => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.refilter();
            }
            _ => {}
        }
        self.ensure_visible(rows);
        None
    }

    fn handle_project_key(&mut self, key: KeyEvent, rows: usize) {
        let Some(projects) = self.project_picker.as_mut() else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.project_picker = None,
            KeyCode::Enter => {
                self.project_filter = projects
                    .visible
                    .get(projects.cursor)
                    .and_then(|&index| projects.options[index].id.clone());
                self.project_picker = None;
                self.refilter();
            }
            KeyCode::Up => projects.move_up(),
            KeyCode::Down => projects.move_down(),
            KeyCode::Backspace => {
                projects.query.pop();
                projects.refilter();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                projects.query.push(character);
                projects.refilter();
            }
            _ => {}
        }
        if let Some(projects) = self.project_picker.as_mut() {
            projects.ensure_visible(rows);
        }
    }

    pub(super) fn current_choice(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    fn move_up(&mut self, amount: usize) {
        if self.visible.is_empty() {
            return;
        }
        self.cursor = if amount == 1 && self.cursor == 0 {
            self.visible.len() - 1
        } else {
            self.cursor.saturating_sub(amount)
        };
    }

    fn move_down(&mut self, amount: usize) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        self.cursor = if amount == 1 && self.cursor == last {
            0
        } else {
            self.cursor.saturating_add(amount).min(last)
        };
    }

    fn refilter(&mut self) {
        let mut scored = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, choice)| {
                (self.query.is_empty() && (self.scope == SessionScope::All || choice.row.running)
                    || !self.query.is_empty())
                    && self
                        .project_filter
                        .as_deref()
                        .is_none_or(|project| choice.row.belongs_to(project))
            })
            .filter_map(|(index, choice)| {
                choice
                    .row
                    .fuzzy_score(&self.query)
                    .map(|score| (index, score))
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

    pub(super) fn replace_choices(&mut self, choices: Vec<SessionChoice>) {
        let selected = self
            .current_choice()
            .map(|index| self.choices[index].row.stable_id());
        self.choices = choices;
        self.refilter();
        if let Some(selected) = selected {
            self.cursor = self
                .visible
                .iter()
                .position(|&index| self.choices[index].row.stable_id() == selected)
                .unwrap_or(0);
        }
        if let Some(projects) = self.project_picker.take() {
            let focused_project = projects
                .visible
                .get(projects.cursor)
                .and_then(|&index| projects.options[index].id.clone());
            let mut refreshed = ProjectPicker::new(&self.choices, self.project_filter.as_deref());
            refreshed.query = projects.query;
            refreshed.refilter();
            if let Some(focused_project) = focused_project {
                refreshed.cursor = refreshed
                    .visible
                    .iter()
                    .position(|&index| {
                        refreshed.options[index].id.as_deref() == Some(focused_project.as_str())
                    })
                    .unwrap_or(0);
            }
            self.project_picker = Some(refreshed);
        }
    }

    pub(super) fn ensure_visible(&mut self, rows: usize) {
        if let Some(projects) = self.project_picker.as_mut() {
            projects.ensure_visible(rows.saturating_mul(2));
            return;
        }
        if rows == 0 || self.visible.is_empty() {
            self.offset = 0;
            return;
        }
        if self.cursor < self.offset {
            self.offset = self.cursor;
        } else if self.cursor >= self.offset + rows {
            self.offset = self.cursor + 1 - rows;
        }
        self.offset = self.offset.min(self.visible.len().saturating_sub(rows));
    }

    pub(super) fn window(&self, rows: usize) -> impl Iterator<Item = (usize, &SessionChoice)> {
        let end = (self.offset + rows).min(self.visible.len());
        self.visible[self.offset..end]
            .iter()
            .enumerate()
            .map(move |(relative, &choice)| (self.offset + relative, &self.choices[choice]))
    }

    pub(super) fn project_label(&self) -> &str {
        let Some(id) = self.project_filter.as_deref() else {
            return "All";
        };
        self.choices
            .iter()
            .flat_map(|choice| choice.row.workspaces.iter())
            .find(|workspace| workspace.id == id)
            .map(|workspace| workspace.name.as_str())
            .unwrap_or(id)
    }
}
