//! Terminal-state guard for the onboarding TUI.

/// RAII guard: enters the alternate screen + raw mode, and always restores the
/// terminal on drop (including during panic unwind).
pub(super) struct TuiTerminal;

impl TuiTerminal {
    pub(super) fn enter() -> std::io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::EnterAlternateScreen,
            crossterm::cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        );
    }
}
