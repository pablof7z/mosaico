//! Onboarding state machine: steps, collected decisions, and key handling.
//!
//! The model is pure: `handle_key` mutates state and returns an [`Action`] the
//! runner performs (network probe, commit, quit). No I/O happens here.

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nostr::{Keys, ToBech32};

use super::super::config::Harness;

const DEVICE_NAME_CAP: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Step {
    Splash,
    Identity,
    DeviceName,
    Harnesses,
    Relay,
    RelayUrl,
    Deploy,
    Review,
}

/// A reasonable default URL to suggest when the operator plans to run their own
/// Croissant relay on this machine.
pub(super) const SUGGESTED_RELAY: &str = "ws://127.0.0.1:9888";

/// The relay branches offered during onboarding. Mosaico does not run a relay;
/// every branch points it at an externally operated NIP-29 relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelayChoice {
    Existing,
    Assist,
    Manual,
}

impl RelayChoice {
    pub(super) const ALL: [RelayChoice; 3] = [
        RelayChoice::Existing,
        RelayChoice::Assist,
        RelayChoice::Manual,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            RelayChoice::Existing => "Connect an existing relay",
            RelayChoice::Assist => "Set one up with an agent",
            RelayChoice::Manual => "Run a Croissant relay myself",
        }
    }

    pub(super) fn blurb(self) -> &'static str {
        match self {
            RelayChoice::Existing => "Point Mosaico at a NIP-29 relay you already operate.",
            RelayChoice::Assist => "An agent helps you stand one up, right in this panel.",
            RelayChoice::Manual => "I'll run Croissant; show me how and wait for it.",
        }
    }
}

/// Live status of the existing-relay verification.
#[derive(Debug, Clone)]
pub(super) enum RelayStatus {
    Idle,
    Verifying,
    Usable,
    Warn(String),
    Failed(String),
}

/// A side effect the runner must perform after a key was handled.
pub(super) enum Action {
    None,
    ProbeRelay(String),
    StartDeploy(String),
    Commit,
    Quit,
}

/// The generated operator identity, shown once and persisted to `config.json`.
pub(super) struct Identity {
    pub nsec: String,
    pub npub: String,
    pub pubkey_hex: String,
}

fn generate_identity() -> Result<Identity> {
    let keys = Keys::generate();
    Ok(Identity {
        nsec: keys
            .secret_key()
            .to_bech32()
            .context("encoding operator nsec")?,
        npub: keys
            .public_key()
            .to_bech32()
            .context("encoding operator npub")?,
        pubkey_hex: keys.public_key().to_hex(),
    })
}

/// Default device name: the slugified hostname, capped and hyphen-trimmed.
fn default_device_name() -> String {
    let slug = crate::slug::slugify_host(&crate::config::hostname());
    let capped: String = slug.chars().take(DEVICE_NAME_CAP).collect();
    let trimmed = capped.trim_end_matches('-');
    if trimmed.is_empty() {
        "mosaico".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) struct Onboarding {
    pub step: Step,
    pub frame: u64,
    pub reduced: bool,
    pub all: Vec<Harness>,
    pub selected: Vec<bool>,
    pub cursor: usize,
    pub identity: Identity,
    pub device_name: String,
    pub relay_cursor: usize,
    pub relay_url: String,
    pub relay_status: RelayStatus,
    pub quit: bool,
}

impl Onboarding {
    pub(super) fn new(all: Vec<Harness>) -> Result<Self> {
        let selected = all.iter().map(|h| h.detected).collect();
        Ok(Self {
            step: Step::Splash,
            frame: 0,
            reduced: super::theme::reduced_motion(),
            selected,
            cursor: 0,
            identity: generate_identity()?,
            device_name: default_device_name(),
            relay_cursor: 0,
            relay_url: String::new(),
            relay_status: RelayStatus::Idle,
            quit: false,
            all,
        })
    }

    pub(super) fn relay_choice(&self) -> RelayChoice {
        RelayChoice::ALL[self.relay_cursor.min(RelayChoice::ALL.len() - 1)]
    }

    /// The first selected harness that can host the structured assist modal.
    pub(super) fn assistable_harness(&self) -> Option<&'static str> {
        self.all
            .iter()
            .zip(&self.selected)
            .filter(|(_, on)| **on)
            .map(|(h, _)| h.id)
            .find(|id| super::deploy::can_assist(id))
    }

    /// Called when a spawned relay probe resolves.
    pub(super) fn on_probe(&mut self, probe: super::relay::Probe) {
        use super::relay::Probe;
        self.relay_status = match probe {
            Probe::Usable => {
                self.step = Step::Review;
                RelayStatus::Usable
            }
            Probe::MissingNip29 => {
                RelayStatus::Warn("reachable, but it does not announce NIP-29".into())
            }
            Probe::Unreachable(msg) => RelayStatus::Failed(msg),
        };
    }

    pub(super) fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return Action::Quit;
        }
        match self.step {
            Step::Splash => self.advance(Step::Identity),
            Step::Identity => self.key_identity(key),
            Step::DeviceName => self.key_device_name(key),
            Step::Harnesses => self.key_harnesses(key),
            Step::Relay => self.key_relay(key),
            Step::RelayUrl => self.key_relay_url(key),
            // The assist modal owns its own keys via DeploySession.
            Step::Deploy => Action::None,
            Step::Review => self.key_review(key),
        }
    }

    fn advance(&mut self, to: Step) -> Action {
        self.step = to;
        Action::None
    }

    fn key_identity(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => self.advance(Step::DeviceName),
            KeyCode::Esc => Action::Quit,
            _ => Action::None,
        }
    }

    fn key_device_name(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter if !self.device_name.trim().is_empty() => self.advance(Step::Harnesses),
            KeyCode::Esc => self.advance(Step::Identity),
            KeyCode::Backspace => {
                self.device_name.pop();
                Action::None
            }
            KeyCode::Char(c)
                if !c.is_control() && self.device_name.chars().count() < DEVICE_NAME_CAP =>
            {
                self.device_name.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn key_harnesses(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.all.len() {
                    self.cursor += 1;
                }
                Action::None
            }
            KeyCode::Char(' ') => {
                if let Some(flag) = self.selected.get_mut(self.cursor) {
                    *flag = !*flag;
                }
                Action::None
            }
            KeyCode::Enter => self.advance(Step::Relay),
            KeyCode::Esc => self.advance(Step::DeviceName),
            _ => Action::None,
        }
    }

    fn key_relay(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.relay_cursor = self.relay_cursor.saturating_sub(1);
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.relay_cursor + 1 < RelayChoice::ALL.len() {
                    self.relay_cursor += 1;
                }
                Action::None
            }
            KeyCode::Enter => {
                self.relay_status = RelayStatus::Idle;
                match self.relay_choice() {
                    RelayChoice::Assist if self.assistable_harness().is_none() => {
                        self.relay_status = RelayStatus::Failed(
                            "pick Claude, Codex, OpenCode, Goose, or Hermes to use an agent".into(),
                        );
                        Action::None
                    }
                    RelayChoice::Assist | RelayChoice::Manual => {
                        if self.relay_url.trim().is_empty() {
                            self.relay_url = SUGGESTED_RELAY.to_string();
                        }
                        self.advance(Step::RelayUrl)
                    }
                    RelayChoice::Existing => self.advance(Step::RelayUrl),
                }
            }
            KeyCode::Esc => self.advance(Step::Harnesses),
            _ => Action::None,
        }
    }

    fn key_relay_url(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => {
                let url = self.relay_url.trim().to_string();
                if url.is_empty() {
                    self.relay_status = RelayStatus::Failed("enter a relay URL".into());
                    return Action::None;
                }
                match self.relay_choice() {
                    // The relay isn't up yet; accept the URL and verify at commit.
                    RelayChoice::Manual => {
                        self.relay_status = RelayStatus::Idle;
                        self.advance(Step::Review)
                    }
                    RelayChoice::Assist => {
                        self.relay_status = RelayStatus::Idle;
                        self.step = Step::Deploy;
                        Action::StartDeploy(url)
                    }
                    RelayChoice::Existing => {
                        self.relay_status = RelayStatus::Verifying;
                        Action::ProbeRelay(url)
                    }
                }
            }
            KeyCode::Esc => self.advance(Step::Relay),
            KeyCode::Backspace => {
                self.relay_url.pop();
                Action::None
            }
            KeyCode::Char(c) if !c.is_control() => {
                self.relay_url.push(c);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn key_review(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Enter => Action::Commit,
            KeyCode::Esc => self.advance(Step::Relay),
            _ => Action::None,
        }
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
