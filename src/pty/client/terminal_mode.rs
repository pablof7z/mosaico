use anyhow::{Context, Result};
use crossterm::terminal;
use std::io::Write;

pub(super) struct TerminalMode;

impl TerminalMode {
    pub(super) fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("enabling raw terminal mode")?;
        Ok(Self)
    }
}

impl Drop for TerminalMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        restore_local_terminal_modes();
    }
}

fn restore_local_terminal_modes() {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(
        b"\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\
          \x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?2004l\
          \x1b[?1016l\x1b[?1007l\x1b[?2026l\x1b[?1049l\
          \x1b[?1047l\x1b[?47l\x1b[?25h\x1b[>4;0m\x1b[<u",
    );
    let _ = stdout.flush();
}
