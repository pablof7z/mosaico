use super::*;

pub const RELAY_EVENT_RETENTION_SECS: u64 = 14 * 24 * 60 * 60;
pub const COMPLETED_LEDGER_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RetentionPruneReport {
    pub relay_events: usize,
    pub delivered_inbox: usize,
    pub completed_event_claims: usize,
    pub native_turn_attempts: usize,
}

impl RetentionPruneReport {
    pub fn total(self) -> usize {
        self.relay_events
            + self.delivered_inbox
            + self.completed_event_claims
            + self.native_turn_attempts
    }
}

impl Store {
    pub fn prune_retained_state(&self, now: u64) -> Result<RetentionPruneReport> {
        self.prune_retained_state_before(
            now.saturating_sub(RELAY_EVENT_RETENTION_SECS),
            now.saturating_sub(COMPLETED_LEDGER_RETENTION_SECS),
        )
    }

    pub fn prune_retained_state_before(
        &self,
        relay_events_before: u64,
        completed_ledgers_before: u64,
    ) -> Result<RetentionPruneReport> {
        let relay_events = self.conn.execute(
            "DELETE FROM relay_events WHERE created_at < ?1",
            params![relay_events_before],
        )?;
        let delivered_inbox = self.conn.execute(
            "DELETE FROM inbox
             WHERE state IN (
                 'delivered', 'injected', 'echo_consumed'
             )
               AND delivered_at > 0 AND delivered_at < ?1",
            params![completed_ledgers_before],
        )?;
        // Offline mentions can start processes, and Nostr observations can
        // replay indefinitely. Their compact completed rows are durable
        // idempotency tombstones, not expiring operational ledgers.
        let completed_event_claims = self.conn.execute(
            "DELETE FROM event_claims
             WHERE state='completed' AND updated_at>0 AND updated_at<?1
               AND claim_key NOT LIKE ?2",
            params![
                completed_ledgers_before,
                format!("{}%", super::event_claims::OFFLINE_MENTION_CLAIM_PREFIX)
            ],
        )?;
        let native_turn_attempts = self.conn.execute(
            "DELETE FROM native_turn_attempts
             WHERE finished_at>0 AND finished_at<?1",
            params![completed_ledgers_before],
        )?;
        Ok(RetentionPruneReport {
            relay_events,
            delivered_inbox,
            completed_event_claims,
            native_turn_attempts,
        })
    }
}
