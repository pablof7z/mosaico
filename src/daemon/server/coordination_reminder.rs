use super::DaemonState;

impl DaemonState {
    pub(crate) fn coordination_reminder_due(&self, pubkey: &str, turn_count: u64) -> bool {
        self.runtime
            .hook_contexts
            .lock()
            .expect("hook-context mutex poisoned")
            .entry(pubkey.to_string())
            .or_default()
            .coordination_reminder_due(turn_count)
    }

    pub(crate) fn record_coordination_reminder(&self, pubkey: &str, turn_count: u64) {
        self.runtime
            .hook_contexts
            .lock()
            .expect("hook-context mutex poisoned")
            .entry(pubkey.to_string())
            .or_default()
            .record_coordination_reminder(turn_count);
    }

    pub(super) fn record_coordination_action(&self, rec: &crate::state::Session) {
        self.runtime
            .hook_contexts
            .lock()
            .expect("hook-context mutex poisoned")
            .entry(rec.pubkey.clone())
            .or_default()
            .record_coordination_action(rec.turn_count.max(1));
    }
}
