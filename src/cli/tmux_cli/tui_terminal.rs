/// RAII guard for raw mode + alternate screen
use anyhow::Result;
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;

/// RAII guard for raw mode + alternate screen. Used to suspend/resume when
/// handing off the tty to a `tmux attach-session` child.
pub(super) struct TuiTerminal;

impl TuiTerminal {
    pub(super) fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }

    /// Temporarily restore the normal terminal so a child process (e.g. a tmux
    /// client) can own the tty, without dropping our guard. Pair with `resume`.
    pub(super) fn suspend() {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }

    /// Re-enter the alternate-screen raw-mode TUI after a `suspend`.
    pub(super) fn resume() {
        let _ = terminal::enable_raw_mode();
        let _ = execute!(io::stdout(), EnterAlternateScreen, Hide);
    }
}

impl Drop for TuiTerminal {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
    }
}
