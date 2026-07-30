//! UI-side controller for the relay-assist modal: owns the live driver, folds
//! its events into a [`Transcript`], and mediates human permission decisions.

use crossterm::event::{KeyCode, KeyEvent};

use super::driver::{self, Driver};
use super::harness::DeployTarget;
use super::transcript::{DeployStatus, Entry, Transcript};
use super::PermissionAsk;

/// Terminal state of the assist session, read by the onboarding loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::cli::install::onboarding) enum DeployOutcome {
    Running,
    Succeeded,
    Cancelled,
}

pub(in crate::cli::install::onboarding) struct DeploySession {
    driver: Driver,
    transcript: Transcript,
    pending: Option<PermissionAsk>,
    option_cursor: usize,
    relay_url: String,
    outcome: DeployOutcome,
}

impl DeploySession {
    pub(in crate::cli::install::onboarding) fn start(
        handle: &tokio::runtime::Handle,
        target: DeployTarget,
        relay_url: String,
        owner_pubkey: String,
    ) -> Self {
        let driver = driver::start(handle, target, relay_url.clone(), owner_pubkey);
        Self {
            driver,
            transcript: Transcript::new(),
            pending: None,
            option_cursor: 0,
            relay_url,
            outcome: DeployOutcome::Running,
        }
    }

    /// Build a session with a seeded transcript and no backing task, for tests.
    #[cfg(test)]
    pub(in crate::cli::install::onboarding) fn for_preview(
        transcript: Transcript,
        pending: Option<PermissionAsk>,
        relay_url: &str,
    ) -> Self {
        Self {
            driver: Driver::disconnected(),
            transcript,
            pending,
            option_cursor: 0,
            relay_url: relay_url.to_string(),
            outcome: DeployOutcome::Running,
        }
    }

    /// Drain driver channels into the transcript. Called every loop tick.
    pub(in crate::cli::install::onboarding) fn pump(&mut self) {
        while let Ok(event) = self.driver.events.try_recv() {
            self.transcript.apply(event);
        }
        if self.pending.is_none() {
            if let Ok(ask) = self.driver.asks.try_recv() {
                self.option_cursor = ask.options.iter().position(|o| o.allow).unwrap_or(0);
                self.pending = Some(ask);
                self.transcript.set_awaiting_permission(true);
            }
        }
    }

    /// The relay probe succeeded — terminal success.
    pub(in crate::cli::install::onboarding) fn relay_online(&mut self) {
        self.transcript.relay_online();
        self.outcome = DeployOutcome::Succeeded;
    }

    pub(in crate::cli::install::onboarding) fn outcome(&self) -> DeployOutcome {
        self.outcome
    }

    pub(in crate::cli::install::onboarding) fn relay_url(&self) -> &str {
        &self.relay_url
    }

    pub(in crate::cli::install::onboarding) fn status(&self) -> &DeployStatus {
        &self.transcript.status
    }

    pub(in crate::cli::install::onboarding) fn entries(&self) -> &[Entry] {
        &self.transcript.entries
    }

    pub(in crate::cli::install::onboarding) fn pending(&self) -> Option<&PermissionAsk> {
        self.pending.as_ref()
    }

    pub(in crate::cli::install::onboarding) fn option_cursor(&self) -> usize {
        self.option_cursor
    }

    pub(in crate::cli::install::onboarding) fn handle_key(&mut self, key: KeyEvent) {
        if self.pending.is_some() {
            self.handle_permission_key(key);
        } else if let KeyCode::Esc = key.code {
            self.driver.cancel();
            self.outcome = DeployOutcome::Cancelled;
        }
    }

    fn handle_permission_key(&mut self, key: KeyEvent) {
        let count = self.pending.as_ref().map(|a| a.options.len()).unwrap_or(0);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.option_cursor = self.option_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if self.option_cursor + 1 < count => {
                self.option_cursor += 1;
            }
            KeyCode::Enter => self.answer(true),
            KeyCode::Esc | KeyCode::Char('n') => self.answer(false),
            _ => {}
        }
    }

    /// Resolve the pending permission: `allow` picks the highlighted option,
    /// otherwise deny (the engine replies "cancelled").
    fn answer(&mut self, allow: bool) {
        if let Some(ask) = self.pending.take() {
            let choice = if allow {
                ask.options.get(self.option_cursor).map(|o| o.id.clone())
            } else {
                None
            };
            let label = match &choice {
                Some(id) => ask
                    .options
                    .iter()
                    .find(|o| &o.id == id)
                    .map(|o| o.label.clone())
                    .unwrap_or_else(|| id.clone()),
                None => "denied".to_string(),
            };
            let _ = ask.respond.send(choice);
            self.transcript
                .apply(super::transcript::DeployEvent::Notice(format!(
                    "permission: {label}"
                )));
            self.transcript.set_awaiting_permission(false);
        }
    }
}
