//! Durable replay guards for backend-owned side effects.
//!
//! These claims are deliberately separate from agent inbox delivery: a claim
//! key identifies an operation target, never a session or agent identity.
//!
//! States: `processing` (claimed, lease `PROCESSING_LEASE_SECS`), `pending`
//! (failed, reclaimable at `retry_at`), `completed`, and terminal `failed`.
//! Retry pacing lives in `retry`.

use super::*;

#[path = "event_claims/ownership.rs"]
mod ownership;
#[path = "event_claims/retry.rs"]
mod retry;
#[cfg(test)]
use retry::RETRY_BASE_DELAY_SECS;
pub(crate) use retry::{ClaimRetry, ORCHESTRATION_CLAIM_PREFIX, SESSION_DISPATCH_CLAIM_PREFIX};

const PROCESSING_LEASE_SECS: u64 = 10 * 60;
pub(super) const OFFLINE_MENTION_CLAIM_PREFIX: &str = "offline-mention:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfflineMentionClaim {
    pub(crate) event_id: String,
    pub(crate) mentioned_pubkey: String,
    pub(crate) from_pubkey: String,
    pub(crate) channel_h: String,
    pub(crate) body: String,
}

impl Store {
    /// Claim one operation target for processing. Completed, terminally
    /// failed, and live in-flight claims return `false`. A `pending` target is
    /// claimable again only at its `retry_at`, so a relay replay of the
    /// retained event cannot retry it in a hot loop.
    pub fn claim_orchestration_target(
        &self,
        event_id: &str,
        claim_key: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<bool> {
        let stale_before = now.saturating_sub(PROCESSING_LEASE_SECS);
        // Taking over an expired lease charges the crashed run's attempt; the
        // last allowed attempt goes terminal instead of running again.
        let exhausted = self.conn.execute(
            "UPDATE event_claims SET state='failed', attempts=attempts+1, updated_at=?3
             WHERE event_id=?1 AND claim_key=?2 AND state='processing'
               AND updated_at < ?4 AND attempts + 1 >= ?5",
            params![
                event_id,
                claim_key,
                now,
                stale_before,
                retry::MAX_RETRY_ATTEMPTS
            ],
        )?;
        if exhausted > 0 {
            retry::log_terminal(
                event_id,
                claim_key,
                Some(retry::MAX_RETRY_ATTEMPTS),
                "processing lease expired on its last attempt",
            );
            return Ok(false);
        }
        let n = self.conn.execute(
            "INSERT INTO event_claims
                 (event_id, claim_key, state, from_pubkey, channel_h, body, created_at, updated_at)
             VALUES (?1, ?2, 'processing', ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(event_id, claim_key) DO UPDATE SET
                 state='processing',
                 from_pubkey=excluded.from_pubkey,
                 channel_h=excluded.channel_h,
                 body=excluded.body,
                 attempts=event_claims.attempts
                     + (CASE event_claims.state WHEN 'processing' THEN 1 ELSE 0 END),
                 updated_at=excluded.updated_at
             WHERE (event_claims.state='pending' AND event_claims.retry_at<=?6)
                OR (event_claims.state='processing' AND event_claims.updated_at < ?7)",
            params![
                event_id,
                claim_key,
                from_pubkey,
                channel_h,
                body,
                now,
                stale_before,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn complete_orchestration_target(
        &self,
        event_id: &str,
        claim_key: &str,
        now: u64,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE event_claims SET state='completed', updated_at=?3
             WHERE event_id=?1 AND claim_key=?2",
            params![event_id, claim_key, now],
        )?;
        Ok(())
    }

    pub fn claim_management_command(
        &self,
        event_id: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<bool> {
        self.claim_orchestration_target(event_id, "management", from_pubkey, channel_h, body, now)
    }

    pub fn complete_management_command(&self, event_id: &str, now: u64) -> Result<()> {
        self.complete_orchestration_target(event_id, "management", now)
    }

    #[cfg(test)]
    pub fn claim_offline_mention(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<bool> {
        Ok(self
            .claim_offline_mention_run(
                event_id,
                mentioned_pubkey,
                from_pubkey,
                channel_h,
                body,
                now,
            )?
            .is_some())
    }

    /// Claim an offline mention for one recovery run; `Some(generation)` when
    /// this call now owns it (see `owns_offline_mention`).
    pub(crate) fn claim_offline_mention_run(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        from_pubkey: &str,
        channel_h: &str,
        body: &str,
        now: u64,
    ) -> Result<Option<u32>> {
        self.claim_driven_target(
            event_id,
            &offline_mention_claim(mentioned_pubkey),
            from_pubkey,
            channel_h,
            body,
            now,
        )
    }

    /// Whether the recovery run that claimed at `generation` still owns it.
    pub(crate) fn owns_offline_mention(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        generation: u32,
    ) -> Result<bool> {
        self.owns_claim(
            event_id,
            &offline_mention_claim(mentioned_pubkey),
            generation,
        )
    }

    /// Record a recovery run's success unless a newer run holds the claim.
    pub(crate) fn complete_offline_mention_run(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        generation: u32,
        now: u64,
    ) -> Result<bool> {
        // One statement: the ownership check, the completion, and the compact
        // tombstone (see `complete_offline_mention`) land together.
        Ok(self.conn.execute(
            "UPDATE event_claims
             SET state='completed', from_pubkey='', channel_h='', body='', updated_at=?4
             WHERE event_id=?1 AND claim_key=?2
               AND ((state='processing' AND attempts=?3) OR state IN ('pending', 'failed'))",
            params![
                event_id,
                offline_mention_claim(mentioned_pubkey),
                generation,
                now
            ],
        )? > 0)
    }

    pub fn complete_offline_mention(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        now: u64,
    ) -> Result<()> {
        // Nostr observations may replay an event long after the local message
        // cache is pruned. Keep a compact tombstone for this process-spawning
        // side effect instead of applying the generic completed-ledger TTL.
        self.conn.execute(
            "UPDATE event_claims
             SET state='completed', from_pubkey='', channel_h='', body='', updated_at=?3
             WHERE event_id=?1 AND claim_key=?2",
            params![event_id, offline_mention_claim(mentioned_pubkey), now],
        )?;
        Ok(())
    }

    /// Return a claimed offline mention to the retryable state. Recovery owns
    /// the side effect; inbox delivery remains independently pending under the
    /// exact addressed pubkey.
    pub(crate) fn retry_offline_mention(
        &self,
        event_id: &str,
        mentioned_pubkey: &str,
        now: u64,
    ) -> Result<Option<ClaimRetry>> {
        self.retry_orchestration_target(event_id, &offline_mention_claim(mentioned_pubkey), now)
    }

    /// Bounded durable recovery work. Failures become eligible at `retry_at`;
    /// abandoned processing leases become eligible after the crash lease.
    pub(crate) fn list_retryable_offline_mentions(
        &self,
        now: u64,
        limit: u32,
    ) -> Result<Vec<OfflineMentionClaim>> {
        let stale_before = now.saturating_sub(PROCESSING_LEASE_SECS);
        let prefix = OFFLINE_MENTION_CLAIM_PREFIX;
        let mut statement = self.conn.prepare(
            "SELECT event_id, substr(claim_key, length(?1) + 1),
                    from_pubkey, channel_h, body
             FROM event_claims
             WHERE claim_key LIKE ?2
               AND ((state='pending' AND retry_at<=?3)
                 OR (state='processing' AND updated_at<?4))
             ORDER BY CASE state WHEN 'pending' THEN retry_at ELSE updated_at + ?6 + 1 END,
                      event_id, claim_key
             LIMIT ?5",
        )?;
        let rows = statement.query_map(
            params![
                prefix,
                format!("{prefix}%"),
                now,
                stale_before,
                limit,
                PROCESSING_LEASE_SECS
            ],
            |row| {
                Ok(OfflineMentionClaim {
                    event_id: row.get(0)?,
                    mentioned_pubkey: row.get(1)?,
                    from_pubkey: row.get(2)?,
                    channel_h: row.get(3)?,
                    body: row.get(4)?,
                })
            },
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn offline_mention_claim(mentioned_pubkey: &str) -> String {
    format!("{OFFLINE_MENTION_CLAIM_PREFIX}{mentioned_pubkey}")
}

#[cfg(test)]
mod tests;
