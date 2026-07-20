use super::{PickerExit, PickerState};
use crate::cli::interactive::agent_picker::DeleteScope;
use crate::cli::interactive::session_picker::HomeChoice;
use crossterm::event::{KeyCode, KeyEvent};

#[derive(Debug)]
pub(super) enum PendingDelete {
    Nothing { slug: String },
    ChooseScope { index: usize },
    Confirm { plan: Vec<(usize, DeleteScope)> },
}

impl PickerState {
    pub(super) fn toggle_agent_selection(&mut self) {
        let Some(index) = self.current_agent_index() else {
            self.notice = Some("Selection is available on launchable agent rows".into());
            return;
        };
        let slug = self.agent(index).slug.clone();
        if !self.selected_agents.remove(&slug) {
            self.selected_agents.insert(slug);
        }
    }

    pub(super) fn begin_delete(&mut self) {
        if self.selected_agents.is_empty() {
            let Some(index) = self.current_agent_index() else {
                self.notice = Some("Use Shift+K to kill a session".into());
                return;
            };
            let row = self.agent(index);
            self.pending_delete = Some(match (row.has_configured(), row.has_native_profile()) {
                (false, false) => PendingDelete::Nothing {
                    slug: row.slug.clone(),
                },
                (true, false) => PendingDelete::Confirm {
                    plan: vec![(index, DeleteScope::Agent)],
                },
                (false, true) => PendingDelete::Confirm {
                    plan: vec![(index, DeleteScope::Profile)],
                },
                (true, true) => PendingDelete::ChooseScope { index },
            });
            return;
        }
        let plan = self
            .choices
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| match choice {
                HomeChoice::Agent(row)
                    if self.selected_agents.contains(&row.slug)
                        && (row.has_configured() || row.has_native_profile()) =>
                {
                    Some((index, DeleteScope::Both))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        self.pending_delete = Some(if plan.is_empty() {
            PendingDelete::Nothing {
                slug: self
                    .selected_agents
                    .first()
                    .expect("checked non-empty above")
                    .clone(),
            }
        } else {
            PendingDelete::Confirm { plan }
        });
    }

    pub(super) fn handle_delete_key(&mut self, key: KeyEvent) -> Option<PickerExit> {
        let pending = self.pending_delete.take().expect("checked by caller");
        match pending {
            PendingDelete::Nothing { .. } => {}
            PendingDelete::ChooseScope { index } => match key.code {
                KeyCode::Char('a') => {
                    self.pending_delete = Some(PendingDelete::Confirm {
                        plan: vec![(index, DeleteScope::Agent)],
                    });
                }
                KeyCode::Char('p') => {
                    self.pending_delete = Some(PendingDelete::Confirm {
                        plan: vec![(index, DeleteScope::Profile)],
                    });
                }
                KeyCode::Char('b') => {
                    self.pending_delete = Some(PendingDelete::Confirm {
                        plan: vec![(index, DeleteScope::Both)],
                    });
                }
                KeyCode::Esc => {}
                _ => self.pending_delete = Some(PendingDelete::ChooseScope { index }),
            },
            PendingDelete::Confirm { plan } => match key.code {
                KeyCode::Char('y' | 'Y' | 'd' | 'D') => {
                    self.selected_agents.clear();
                    return Some(PickerExit::Delete(plan));
                }
                KeyCode::Esc => {}
                _ => self.pending_delete = Some(PendingDelete::Confirm { plan }),
            },
        }
        None
    }
}
