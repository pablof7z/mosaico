use super::{PickerExit, PickerState};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Confirmation {
    TakeOver(usize),
    InterruptWorking(usize),
}

impl PickerState {
    pub(super) fn handle_confirmation(&mut self, key: KeyEvent) -> Option<PickerExit> {
        let confirmation = self.confirmation?;
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(PickerExit::Cancel)
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.confirmation = None;
                None
            }
            KeyCode::Char('y' | 'Y') => match confirmation {
                Confirmation::TakeOver(choice) if self.session(choice).row.turn_open => {
                    self.confirmation = Some(Confirmation::InterruptWorking(choice));
                    None
                }
                Confirmation::TakeOver(choice) => Some(PickerExit::TakeOver(choice, None)),
                Confirmation::InterruptWorking(choice) => Some(PickerExit::TakeOver(
                    choice,
                    Some(self.session(choice).row.turn_count),
                )),
            },
            _ => None,
        }
    }

    pub(super) fn confirmation_text(&self) -> Option<String> {
        match self.confirmation? {
            Confirmation::TakeOver(choice) => Some(format!(
                "[y] take over  [n] cancel · Kill @{}, resume it in a managed PTY, and attach; terminal-only scrollback is lost",
                self.session(choice).row.handle
            )),
            Confirmation::InterruptWorking(choice) => Some(format!(
                "[y] interrupt  [n] cancel · No end-of-turn hook received from @{}; it may still be working",
                self.session(choice).row.handle
            )),
        }
    }
}
