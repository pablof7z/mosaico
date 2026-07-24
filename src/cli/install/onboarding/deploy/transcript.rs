//! Normalized, dialect-agnostic transcript for the relay-assist modal.
//!
//! The driver decodes ACP `session/update` notifications and Codex app-server
//! items into [`DeployEvent`]s; this reducer folds them into a renderable
//! [`Transcript`]. Nothing here does I/O, so it is fully unit-tested.

/// A normalized event, produced from either RPC dialect or from the driver
/// itself (notices/errors). Permission requests travel on a separate channel
/// because they carry a responder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli::install::onboarding) enum DeployEvent {
    /// A chunk of agent-visible message text (streamed).
    Agent(String),
    /// A chunk of agent reasoning/thinking text (streamed).
    Thought(String),
    /// A one-line summary of tool/command activity.
    Activity(String),
    /// A driver-originated status line (spawned, initialized, turn ended).
    Notice(String),
    /// A driver or turn error.
    Error(String),
    /// The agent's turn finished (informational — success is relay reachability).
    TurnEnded,
}

/// A rendered transcript entry, tagged for styling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli::install::onboarding) enum Entry {
    Agent(String),
    Thought(String),
    Activity(String),
    Notice(String),
    Error(String),
}

/// The high-level state of the assist session, shown in the status strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::cli::install::onboarding) enum DeployStatus {
    /// Spawning / initializing the harness child.
    Connecting,
    /// The agent turn is running.
    Working,
    /// Blocked on a human permission decision.
    AwaitingPermission,
    /// The turn ended; still waiting for the relay to come online.
    Idle,
    /// The relay probe succeeded — the modal can finish.
    RelayOnline,
    /// The session failed; carries a short reason.
    Failed(String),
}

pub(in crate::cli::install::onboarding) struct Transcript {
    pub entries: Vec<Entry>,
    pub status: DeployStatus,
}

impl Transcript {
    pub(in crate::cli::install::onboarding) fn new() -> Self {
        Self {
            entries: Vec::new(),
            status: DeployStatus::Connecting,
        }
    }

    /// Fold one normalized event into the transcript.
    pub(in crate::cli::install::onboarding) fn apply(&mut self, event: DeployEvent) {
        match event {
            DeployEvent::Agent(text) => {
                self.stream_into(text, EntryKind::Agent);
                self.working();
            }
            DeployEvent::Thought(text) => {
                self.stream_into(text, EntryKind::Thought);
                self.working();
            }
            DeployEvent::Activity(text) => {
                self.entries.push(Entry::Activity(text));
                self.working();
            }
            DeployEvent::Notice(text) => self.entries.push(Entry::Notice(text)),
            DeployEvent::Error(text) => {
                self.entries.push(Entry::Error(text.clone()));
                self.status = DeployStatus::Failed(text);
            }
            DeployEvent::TurnEnded => {
                if !matches!(
                    self.status,
                    DeployStatus::Failed(_) | DeployStatus::RelayOnline
                ) {
                    self.status = DeployStatus::Idle;
                }
            }
        }
    }

    /// Mark the relay verified — terminal success.
    pub(in crate::cli::install::onboarding) fn relay_online(&mut self) {
        self.status = DeployStatus::RelayOnline;
    }

    /// Enter/leave the permission-blocked state without losing a failure.
    pub(in crate::cli::install::onboarding) fn set_awaiting_permission(&mut self, awaiting: bool) {
        match (&self.status, awaiting) {
            (DeployStatus::Failed(_) | DeployStatus::RelayOnline, _) => {}
            (_, true) => self.status = DeployStatus::AwaitingPermission,
            (_, false) => self.status = DeployStatus::Working,
        }
    }

    fn working(&mut self) {
        if matches!(self.status, DeployStatus::Connecting | DeployStatus::Idle) {
            self.status = DeployStatus::Working;
        }
    }

    /// Append streamed text to the last entry of the same kind, else start one.
    fn stream_into(&mut self, text: String, kind: EntryKind) {
        match (self.entries.last_mut(), kind) {
            (Some(Entry::Agent(prev)), EntryKind::Agent) => prev.push_str(&text),
            (Some(Entry::Thought(prev)), EntryKind::Thought) => prev.push_str(&text),
            (_, EntryKind::Agent) => self.entries.push(Entry::Agent(text)),
            (_, EntryKind::Thought) => self.entries.push(Entry::Thought(text)),
        }
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    Agent,
    Thought,
}

#[cfg(test)]
#[path = "transcript_tests.rs"]
mod tests;
