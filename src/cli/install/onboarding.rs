//! Full-screen first-run onboarding TUI for `mosaico setup`.
//!
//! The interactive default when `mosaico setup` is run on a TTY with no
//! scripting flags. It composes a branded opening, a generated operator
//! identity, a device name, harness selection, and a relay branch, then hands
//! the collected decisions to [`commit`] which writes `config.json` and applies
//! the shared install mechanics. Any non-interactive or flag-driven invocation
//! falls back to the scriptable text flow.

mod commit;
mod deploy;
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
use deploy::{DeployOutcome, DeploySession};
use model::{Action, Onboarding, RelayStatus, Step};
use render::TuiTerminal;

const SPLASH_HOLD: Duration = Duration::from_millis(900);
const POLL: Duration = Duration::from_millis(80);
/// Loop ticks (~80ms each) between relay probes while the assist modal runs.
const DEPLOY_PROBE_TICKS: u64 = 25;

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
    let (dtx, drx) = std::sync::mpsc::channel::<relay::Probe>();
    let started = Instant::now();
    let mut deploy: Option<DeploySession> = None;

    loop {
        terminal.draw(|frame| match (state.step, &deploy) {
            (Step::Deploy, Some(session)) => deploy::draw_modal(frame, session),
            _ => render::draw(frame, state),
        })?;

        while let Ok(probe) = rx.try_recv() {
            state.on_probe(probe);
        }

        if state.step == Step::Splash && started.elapsed() >= SPLASH_HOLD {
            state.step = Step::Identity;
        }

        // Drive the assist modal: pump the driver, poll the relay, resolve.
        if let Some(session) = deploy.as_mut() {
            session.pump();
            if state.frame % DEPLOY_PROBE_TICKS == 0 {
                let dtx = dtx.clone();
                let url = session.relay_url().to_string();
                handle.spawn(async move {
                    let _ = dtx.send(relay::probe(&url).await);
                });
            }
            if let Ok(relay::Probe::Usable) = drx.try_recv() {
                session.relay_online();
            }
            match session.outcome() {
                DeployOutcome::Succeeded => {
                    state.relay_url = session.relay_url().to_string();
                    state.step = Step::Review;
                    deploy = None;
                }
                DeployOutcome::Cancelled => {
                    state.step = Step::Relay;
                    state.relay_status = RelayStatus::Idle;
                    deploy = None;
                }
                DeployOutcome::Running => {}
            }
        }

        if event::poll(POLL)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(session) = deploy.as_mut() {
                        session.handle_key(key);
                    } else {
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
                            Action::StartDeploy(url) => {
                                deploy = start_deploy(state, handle, url);
                            }
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

/// Resolve the assist harness and spawn its session, or drop back to the relay
/// branch with a message if no harness can host it.
fn start_deploy(
    state: &mut Onboarding,
    handle: &tokio::runtime::Handle,
    url: String,
) -> Option<DeploySession> {
    let Some(id) = state.assistable_harness() else {
        state.step = Step::Relay;
        state.relay_status = RelayStatus::Failed("no agent-capable harness selected".into());
        return None;
    };
    match deploy::resolve(id) {
        Ok(target) => Some(DeploySession::start(
            handle,
            target,
            url,
            state.identity.pubkey_hex.clone(),
        )),
        Err(e) => {
            state.step = Step::Relay;
            state.relay_status = RelayStatus::Failed(format!("could not start agent: {e}"));
            None
        }
    }
}
