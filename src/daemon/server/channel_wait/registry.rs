//! Ephemeral observations of waits whose RPC futures are currently alive.

use super::super::DaemonState;
use super::AuthorFilter;
use crate::state::Session;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(in crate::daemon::server) struct ActiveWaitRegistry {
    next_id: u64,
    waits: HashMap<u64, ActiveWait>,
}

struct ActiveWait {
    pubkey: String,
    runtime_generation: u64,
    scopes: HashSet<String>,
    expected_authors: Option<HashSet<String>>,
    correlated_reply_to: Option<String>,
}

impl ActiveWaitRegistry {
    fn insert(&mut self, wait: ActiveWait) -> u64 {
        self.next_id = self.next_id.saturating_add(1).max(1);
        let id = self.next_id;
        self.waits.insert(id, wait);
        id
    }

    pub(super) fn matches(
        &self,
        session: &Session,
        channel: &str,
        expected_authors: &[String],
    ) -> bool {
        !expected_authors.is_empty()
            && self.waits.values().any(|wait| {
                wait.pubkey == session.pubkey
                    && wait.runtime_generation == session.runtime_generation
                    && wait.correlated_reply_to.is_none()
                    && wait.scopes.contains(channel)
                    && wait.expected_authors.as_ref().is_none_or(|authors| {
                        expected_authors
                            .iter()
                            .any(|author| authors.contains(author))
                    })
            })
    }
}

impl DaemonState {
    pub(in crate::daemon::server) fn has_matching_active_wait(
        &self,
        session: &Session,
        channel: &str,
        expected_authors: &[String],
    ) -> bool {
        self.runtime
            .active_waits
            .lock()
            .expect("active-wait mutex poisoned")
            .matches(session, channel, expected_authors)
    }
}

pub(super) struct ActiveWaitGuard {
    registry: Arc<Mutex<ActiveWaitRegistry>>,
    id: u64,
}

impl Drop for ActiveWaitGuard {
    fn drop(&mut self) {
        self.registry
            .lock()
            .expect("active-wait mutex poisoned")
            .waits
            .remove(&self.id);
    }
}

pub(super) fn register(
    state: &Arc<DaemonState>,
    session: &Session,
    scopes: &[String],
    author_filter: &AuthorFilter,
    reply_to: Option<&str>,
) -> ActiveWaitGuard {
    let registry = state.runtime.active_waits.clone();
    let wait = ActiveWait {
        pubkey: session.pubkey.clone(),
        runtime_generation: session.runtime_generation,
        scopes: scopes.iter().cloned().collect(),
        expected_authors: author_filter.expected_pubkeys(),
        correlated_reply_to: reply_to.map(str::to_string),
    };
    let id = registry
        .lock()
        .expect("active-wait mutex poisoned")
        .insert(wait);
    ActiveWaitGuard { registry, id }
}
