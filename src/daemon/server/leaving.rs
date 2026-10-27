//! Explicit leaves in flight, shared with standing repair.

use super::*;

/// Marks one (pubkey, channel) leave in flight until dropped.
pub(super) struct LeavingGuard {
    state: Arc<DaemonState>,
    key: (String, String),
}

impl DaemonState {
    pub(super) fn begin_leaving(self: &Arc<Self>, pubkey: &str, channel: &str) -> LeavingGuard {
        let key = (pubkey.to_string(), channel.to_string());
        self.leaving
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(key.clone());
        LeavingGuard {
            state: self.clone(),
            key,
        }
    }

    /// A leave owns this (pubkey, channel) right now; standing repair must
    /// not act on its intermediate state.
    pub(super) fn is_leaving(&self, pubkey: &str, channel: &str) -> bool {
        self.leaving
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .contains(&(pubkey.to_string(), channel.to_string()))
    }
}

impl Drop for LeavingGuard {
    fn drop(&mut self) {
        self.state
            .leaving
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&self.key);
    }
}
