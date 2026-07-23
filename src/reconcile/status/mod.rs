//! Generation-fenced presence-lease publication policy.
//!
//! Managed lifecycle owns session truth. This reconciler only projects a
//! lifecycle snapshot into an expiring signed status lease.

mod command;
mod status_build;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use crate::domain::Status;
use crate::session_state::SessionState;

pub use command::{StatusCommand, StatusOutcome};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishReason {
    Opened,
    Changed,
    Renewed,
}

impl PublishReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Changed => "changed",
            Self::Renewed => "renewed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusEffect {
    Publish {
        status: Status,
        reason: PublishReason,
    },
    Expire {
        status: Status,
    },
}

/// Complete lifecycle-owned input to public presence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceSnapshot {
    pub host: String,
    pub slug: String,
    pub rel_cwd: String,
    pub dispatch_event: Option<String>,
    pub projection: PresenceProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresenceProjection {
    pub channels: BTreeSet<String>,
    pub state: SessionState,
    pub state_since: u64,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PublishedPresence {
    generation: u64,
    snapshot: PresenceSnapshot,
    live: bool,
    renewal_arm: u64,
}

#[derive(Clone)]
pub struct StatusReconciler {
    ttl_secs: u64,
    renewal_secs: u64,
    sessions: BTreeMap<String, PublishedPresence>,
    revision: u64,
}

impl StatusReconciler {
    pub fn new(ttl_secs: u64, renewal_secs: u64) -> Self {
        Self {
            ttl_secs: ttl_secs.max(1),
            renewal_secs: renewal_secs.max(1),
            sessions: BTreeMap::new(),
            revision: 0,
        }
    }

    pub fn for_ttl(ttl: Duration) -> Self {
        Self::new(ttl.as_secs(), crate::domain::PRESENCE_LEASE_RENEWAL_SECS)
    }

    /// Acquire publication ownership for one runtime generation.
    ///
    /// A higher generation replaces prior ownership. Equal and older starts are
    /// idempotent because lifecycle reconciliation has a separate explicit path.
    pub fn open(
        &mut self,
        pubkey: &str,
        generation: u64,
        snapshot: PresenceSnapshot,
        now: u64,
    ) -> StatusOutcome {
        if self
            .sessions
            .get(pubkey)
            .is_some_and(|current| current.generation >= generation)
        {
            return self.empty_outcome(pubkey);
        }
        let state = PublishedPresence {
            generation,
            snapshot,
            live: true,
            renewal_arm: self.renewal_arm(now),
        };
        let status = self.status_of(pubkey, &state, now, false);
        self.sessions.insert(pubkey.to_string(), state);
        self.outcome(
            pubkey,
            vec![StatusEffect::Publish {
                status,
                reason: PublishReason::Opened,
            }],
        )
    }

    /// Publish a semantic lifecycle change for the owning generation.
    pub fn reconcile(
        &mut self,
        pubkey: &str,
        generation: u64,
        projection: PresenceProjection,
        now: u64,
    ) -> StatusOutcome {
        let Some(state) = self.owned_mut(pubkey, generation) else {
            return self.empty_outcome(pubkey);
        };
        let before = command_of(pubkey, state);
        state.snapshot.projection = projection;
        state.live = true;
        let after = command_of(pubkey, state);
        let effects = (after != before)
            .then(|| StatusEffect::Publish {
                status: status_build::to_status(&after, self.ttl_secs, now, false),
                reason: PublishReason::Changed,
            })
            .into_iter()
            .collect();
        self.outcome(pubkey, effects)
    }

    /// Extend freshness only. Renewal never changes semantic content.
    pub fn renew(&mut self, pubkey: &str, generation: u64, now: u64) -> StatusOutcome {
        let arm = self.renewal_arm(now);
        let Some(state) = self.owned_mut(pubkey, generation) else {
            return self.empty_outcome(pubkey);
        };
        if state.renewal_arm == arm {
            return self.empty_outcome(pubkey);
        }
        state.renewal_arm = arm;
        let status = status_build::to_status(&command_of(pubkey, state), self.ttl_secs, now, false);
        self.outcome(
            pubkey,
            vec![StatusEffect::Publish {
                status,
                reason: PublishReason::Renewed,
            }],
        )
    }

    /// Publish immediate offline state for exactly one runtime generation.
    pub fn close(&mut self, pubkey: &str, generation: u64, now: u64) -> StatusOutcome {
        let Some(state) = self.owned_mut(pubkey, generation) else {
            return self.empty_outcome(pubkey);
        };
        if !state.live {
            return self.empty_outcome(pubkey);
        }
        state.live = false;
        let status = status_build::to_status(&command_of(pubkey, state), self.ttl_secs, now, false);
        self.outcome(
            pubkey,
            vec![StatusEffect::Publish {
                status,
                reason: PublishReason::Changed,
            }],
        )
    }

    /// Remove all presence for exactly one revoked runtime generation.
    pub fn revoke(&mut self, pubkey: &str, generation: u64, now: u64) -> StatusOutcome {
        let Some(state) = self
            .sessions
            .get(pubkey)
            .filter(|state| state.generation == generation)
            .cloned()
        else {
            return self.empty_outcome(pubkey);
        };
        self.sessions.remove(pubkey);
        self.outcome(
            pubkey,
            vec![StatusEffect::Expire {
                status: self.status_of(pubkey, &state, now, true),
            }],
        )
    }

    fn owned_mut(&mut self, pubkey: &str, generation: u64) -> Option<&mut PublishedPresence> {
        self.sessions
            .get_mut(pubkey)
            .filter(|state| state.generation == generation)
    }

    fn renewal_arm(&self, now: u64) -> u64 {
        now / self.renewal_secs
    }

    fn status_of(
        &self,
        pubkey: &str,
        state: &PublishedPresence,
        now: u64,
        expiring: bool,
    ) -> Status {
        status_build::to_status(&command_of(pubkey, state), self.ttl_secs, now, expiring)
    }

    fn empty_outcome(&self, pubkey: &str) -> StatusOutcome {
        StatusOutcome {
            effects: Vec::new(),
            revision: self.revision,
            pubkey: Some(pubkey.to_string()),
        }
    }

    fn outcome(&mut self, pubkey: &str, effects: Vec<StatusEffect>) -> StatusOutcome {
        self.revision = self.revision.saturating_add(1);
        StatusOutcome {
            effects,
            revision: self.revision,
            pubkey: Some(pubkey.to_string()),
        }
    }
}

fn command_of(pubkey: &str, state: &PublishedPresence) -> StatusCommand {
    let snapshot = &state.snapshot;
    StatusCommand {
        pubkey: pubkey.to_string(),
        channels: snapshot.projection.channels.iter().cloned().collect(),
        title: snapshot.projection.title.clone(),
        state: if state.live {
            snapshot.projection.state
        } else {
            SessionState::Offline
        },
        state_since: snapshot.projection.state_since,
        host: snapshot.host.clone(),
        slug: snapshot.slug.clone(),
        rel_cwd: snapshot.rel_cwd.clone(),
        dispatch_event: snapshot.dispatch_event.clone(),
    }
}
