//! Full-screen first-run onboarding TUI for `mosaico setup`.
//!
//! The interactive default when `mosaico setup` is run on a TTY with no
//! scripting flags. It composes a branded opening, a generated operator
//! identity, a device name, harness selection, and a relay branch, then hands
//! the collected decisions to [`commit`] which writes `config.json` and applies
//! the shared install mechanics. Any non-interactive or flag-driven invocation
//! falls back to the scriptable text flow.

mod commit;
mod model;
mod relay;
mod render;
mod theme;

use std::io::stdout;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use super::args::InstallOpts;
use model::{Action, Onboarding, Step};
use render::TuiTerminal;

const SPLASH_HOLD: Duration = Duration::from_millis(900);
const POLL: Duration = Duration::from_millis(80);

/// Whether the onboarding TUI should own this `setup` invocation. Any scripting
/// flag or non-TTY context defers to the text flow.
pub(super) fn should_run(opts: &InstallOpts) -> bool {
    use std::io::IsTerminal as _;
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && !opts.status
        && !opts.dry_run
        && !opts.all
        && !opts.uninstall
        && opts.harness.is_none()
        && opts.relay.is_empty()
        && opts.host_label.is_none()
        && opts.operator_pubkeys.is_none()
        && !opts.clear_operators
        && opts.operator_nsec_file.is_none()
        && !opts.clear_operator_nsec
        && opts.indexer_relay.is_none()
        && opts.per_session_rooms.is_none()
}

enum Outcome {
    Commit,
    Quit,
}

pub(super) async fn run(_opts: InstallOpts) -> Result<()> {
    let all = super::config::harnesses()?;
    let mut state = Onboarding::new(all)?;
    if state.reduced {
        // No animation: skip the splash and land on the first decision.
        state.step = Step::Identity;
    }

    let handle = tokio::runtime::Handle::current();
    let outcome = tokio::task::block_in_place(|| tui_loop(&mut state, &handle))?;

    match outcome {
        Outcome::Commit => commit::commit(state).await,
        Outcome::Quit => {
            println!("Setup cancelled — nothing was written.");
            Ok(())
        }
    }
}

fn tui_loop(state: &mut Onboarding, handle: &tokio::runtime::Handle) -> Result<Outcome> {
    let _guard = TuiTerminal::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    let (tx, rx) = std::sync::mpsc::channel::<relay::Probe>();
    let started = Instant::now();

    loop {
        terminal.draw(|frame| render::draw(frame, state))?;

        while let Ok(probe) = rx.try_recv() {
            state.on_probe(probe);
        }

        if state.step == Step::Splash && started.elapsed() >= SPLASH_HOLD {
            state.step = Step::Identity;
        }

        if event::poll(POLL)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match state.handle_key(key) {
                        Action::None => {}
                        Action::Quit => return Ok(Outcome::Quit),
                        Action::Commit => return Ok(Outcome::Commit),
                        Action::ProbeRelay(url) => {
                            let tx = tx.clone();
                            handle.spawn(async move {
                                let _ = tx.send(relay::probe(&url).await);
                            });
                        }
                    }
                }
            }
        }

        state.frame = state.frame.wrapping_add(1);
        if state.quit {
            return Ok(Outcome::Quit);
        }
    }
}
