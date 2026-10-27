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
            .entry(key.clone())
            .and_modify(|count| *count += 1)
            .or_insert(1);
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
            .contains_key(&(pubkey.to_string(), channel.to_string()))
    }
}

impl Drop for LeavingGuard {
    fn drop(&mut self) {
        let mut leaving = self
            .state
            .leaving
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(count) = leaving.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                leaving.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A duplicate leave (a client retry) finishing first must not release
    /// the in-flight mark the original leave still depends on.
    #[tokio::test]
    async fn a_duplicate_leave_finishing_first_keeps_the_original_in_flight() {
        let state = DaemonState::new_for_test().await;
        let original = state.begin_leaving("pk", "room");
        let duplicate = state.begin_leaving("pk", "room");
        drop(duplicate);
        assert!(state.is_leaving("pk", "room"));
        drop(original);
        assert!(!state.is_leaving("pk", "room"));
    }
}
