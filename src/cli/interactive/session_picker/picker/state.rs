use super::{
    confirmation, delete::PendingDelete, project::ProjectPicker, range::HistoryRange, PickerExit,
};
use crate::cli::interactive::session_picker::{HomeChoice, SessionChoice};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::BTreeSet;

mod filter;
mod viewport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PickerTab {
    Sessions,
    Agents,
}

#[derive(Debug)]
pub(super) struct PickerState {
    pub(super) choices: Vec<HomeChoice>,
    pub(super) visible: Vec<usize>,
    pub(super) query: String,
    pub(super) filtering: bool,
    pub(super) range: HistoryRange,
    pub(super) project_filter: Option<String>,
    pub(super) project_picker: Option<ProjectPicker>,
    pub(super) notice: Option<String>,
    pub(super) confirmation: Option<confirmation::Confirmation>,
    pub(super) pending_delete: Option<PendingDelete>,
    pub(super) selected_agents: BTreeSet<String>,
    pub(super) tab: PickerTab,
    pub(super) cursor: usize,
    pub(super) offset: usize,
}

impl PickerState {
    pub(super) fn new(choices: Vec<HomeChoice>, initial_focus: Option<&str>) -> Self {
        let has_sessions = choices.iter().any(HomeChoice::is_session);
        let tab = if initial_focus.is_some_and(|focus| focus.starts_with("agent:")) || !has_sessions
        {
            PickerTab::Agents
        } else {
            PickerTab::Sessions
        };
        let mut state = Self {
            choices,
            visible: Vec::new(),
            query: String::new(),
            filtering: false,
            range: HistoryRange::Live,
            project_filter: None,
            project_picker: None,
            notice: None,
            confirmation: None,
            pending_delete: None,
            selected_agents: BTreeSet::new(),
            tab,
            cursor: 0,
            offset: 0,
        };
        state.refilter();
        if let Some(initial_focus) = initial_focus {
            state.cursor = state
                .visible
                .iter()
                .position(|&index| state.choices[index].stable_id() == initial_focus)
                .unwrap_or(0);
        }
        state
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent, lines: usize) -> Option<PickerExit> {
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if self.project_picker.is_some() {
            self.handle_project_key(key, lines);
            return None;
        }
        if self.confirmation.is_some() {
            return self.handle_confirmation(key);
        }
        if self.pending_delete.is_some() {
            return self.handle_delete_key(key);
        }
        self.notice = None;
        match key.code {
            KeyCode::Esc if self.filtering => {
                self.query.clear();
                self.filtering = false;
                self.refilter();
            }
            KeyCode::Esc => return Some(PickerExit::Cancel),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Some(PickerExit::Cancel);
            }
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right if !self.filtering => {
                self.switch_tab()
            }
            KeyCode::Char('K') | KeyCode::Char('k')
                if key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if let Some(index) = self.current_session_index() {
                    return Some(PickerExit::Kill(index));
                }
                self.notice = Some("Shift+K kills sessions; agent rows are never running".into());
            }
            KeyCode::Char('e') if !self.filtering => {
                if let Some(index) = self.current_agent_index() {
                    return Some(PickerExit::Edit(index));
                }
                self.notice = Some("Edit is available on launchable agent rows".into());
            }
            KeyCode::Char(' ') if !self.filtering => self.toggle_agent_selection(),
            KeyCode::Char('d') if !self.filtering => self.begin_delete(),
            KeyCode::Char('p')
                if !self.filtering
                    && self.tab == PickerTab::Sessions
                    && key.modifiers.is_empty() =>
            {
                self.project_picker = Some(ProjectPicker::new(
                    &self.choices,
                    self.project_filter.as_deref(),
                ));
            }
            KeyCode::Char('+')
                if !self.filtering
                    && self.tab == PickerTab::Sessions
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.range.expand();
                self.refilter();
            }
            KeyCode::Char('-')
                if !self.filtering
                    && self.tab == PickerTab::Sessions
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.range.narrow();
                self.refilter();
            }
            KeyCode::Enter => return self.activate_current(),
            KeyCode::Up => self.move_up(1),
            KeyCode::Down => self.move_down(1),
            KeyCode::PageUp => self.move_up((lines / 2).max(1)),
            KeyCode::PageDown => self.move_down((lines / 2).max(1)),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.visible.len().saturating_sub(1),
            KeyCode::Char('/') if !self.filtering => self.filtering = true,
            KeyCode::Backspace if self.filtering => {
                self.query.pop();
                self.refilter();
            }
            KeyCode::Char(character)
                if self.filtering
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(character);
                self.refilter();
            }
            _ => {}
        }
        self.ensure_visible(lines);
        None
    }

    fn switch_tab(&mut self) {
        self.tab = match self.tab {
            PickerTab::Sessions => PickerTab::Agents,
            PickerTab::Agents => PickerTab::Sessions,
        };
        self.refilter();
    }

    fn activate_current(&mut self) -> Option<PickerExit> {
        let index = self.current_choice()?;
        let HomeChoice::Session(choice) = &self.choices[index] else {
            return Some(PickerExit::Launch(index));
        };
        let row = &choice.row;
        if row.attachable() {
            return Some(PickerExit::Attach(index));
        }
        if row.resumable && !row.running {
            return Some(PickerExit::Resume(index));
        }
        if row.can_take_over() {
            self.confirmation = Some(confirmation::Confirmation::TakeOver(index));
            return None;
        }
        self.notice = Some(if !row.running {
            "This exited session has no restartable harness state".into()
        } else if row.transport == "acp" {
            "ACP sessions run without a harness terminal — nothing to attach to".into()
        } else {
            "This session has no live attachable terminal".into()
        });
        None
    }

    fn handle_project_key(&mut self, key: KeyEvent, lines: usize) {
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
            projects.ensure_visible(lines);
        }
    }

    pub(super) fn current_choice(&self) -> Option<usize> {
        self.visible.get(self.cursor).copied()
    }

    pub(super) fn current_agent_index(&self) -> Option<usize> {
        self.current_choice()
            .filter(|&index| matches!(self.choices[index], HomeChoice::Agent(_)))
    }

    fn current_session_index(&self) -> Option<usize> {
        self.current_choice()
            .filter(|&index| matches!(self.choices[index], HomeChoice::Session(_)))
    }

    pub(super) fn agent(&self, index: usize) -> &crate::cli::agents::AgentRow {
        match &self.choices[index] {
            HomeChoice::Agent(row) => row,
            HomeChoice::Session(_) => unreachable!("agent index targeted a session"),
        }
    }

    pub(super) fn session(&self, index: usize) -> &SessionChoice {
        match &self.choices[index] {
            HomeChoice::Session(choice) => choice,
            HomeChoice::Agent(_) => unreachable!("session index targeted an agent"),
        }
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

    pub(super) fn can_refresh(&self) -> bool {
        self.confirmation.is_none() && self.pending_delete.is_none()
    }
}
