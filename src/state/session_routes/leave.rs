//! Explicit leave as a durable two-phase transition.
//!
//! Phase one removes the route but keeps the relay-confirmed `member`
//! standing. That pair is exactly the durable "relay removal is due" state
//! the standing cleanup already retries, and with the route gone presence no
//! longer projects the channel. Phase two records the confirmed relay
//! removal; a failed removal restores the original route.

use super::*;

/// The route a leave removed, kept so a failed relay removal can restore it
/// exactly (history cursor included), fenced on the state it was taken from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeavingRoute {
    joined_at: u64,
    joined_event_seq: i64,
    runtime_generation: u64,
    standing_epoch: Option<u64>,
}

impl LeavingRoute {
    /// The leave's standing was `member` when it began.
    pub fn was_member(&self) -> bool {
        self.standing_epoch.is_some()
    }
}

impl Store {
    /// Remove `channel_h` from the session's routes while its relay standing
    /// is still `member`. Returns `None` when the session has no such route.
    pub fn begin_route_leave(&self, pubkey: &str, channel_h: &str) -> Result<Option<LeavingRoute>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let route = transaction
            .query_row(
                "SELECT route.joined_at, route.joined_event_seq, session.runtime_generation,
                        (SELECT standing.standing_epoch FROM session_standing standing
                          WHERE standing.pubkey=route.pubkey
                            AND standing.channel_h=route.channel_h
                            AND standing.state='member')
                   FROM session_channels route
                   JOIN sessions session ON session.pubkey=route.pubkey
                  WHERE route.pubkey=?1 AND route.channel_h=?2",
                params![pubkey, channel_h],
                |row| {
                    Ok(LeavingRoute {
                        joined_at: row.get(0)?,
                        joined_event_seq: row.get(1)?,
                        runtime_generation: row.get(2)?,
                        standing_epoch: row.get(3)?,
                    })
                },
            )
            .optional()?;
        if route.is_some() {
            transaction.execute(
                "DELETE FROM session_channels WHERE pubkey=?1 AND channel_h=?2",
                params![pubkey, channel_h],
            )?;
        }
        transaction.commit()?;
        Ok(route)
    }

    /// Restore the route a leave removed because the relay removal failed,
    /// only while nothing newer owns this session and channel: the same
    /// runtime generation, not revoked (forget), no route re-added by a
    /// concurrent join, and the standing epoch the leave began from. Returns
    /// whether the route was restored.
    pub fn restore_route_after_failed_leave(
        &self,
        pubkey: &str,
        channel_h: &str,
        route: &LeavingRoute,
    ) -> Result<bool> {
        Ok(self.conn.execute(
            "INSERT INTO session_channels (pubkey, channel_h, joined_at, joined_event_seq)
             SELECT ?1, ?2, ?3, ?4
              WHERE EXISTS (
                        SELECT 1 FROM sessions
                         WHERE pubkey=?1 AND runtime_generation=?5
                           AND recovery_state<>'revoked')
                AND NOT EXISTS (
                        SELECT 1 FROM session_channels WHERE pubkey=?1 AND channel_h=?2)
                AND (SELECT standing_epoch FROM session_standing
                      WHERE pubkey=?1 AND channel_h=?2 AND state='member') IS ?6",
            params![
                pubkey,
                channel_h,
                route.joined_at,
                route.joined_event_seq,
                route.runtime_generation,
                route.standing_epoch
            ],
        )? == 1)
    }

    /// Record the relay-confirmed removal of a left channel.
    pub fn finish_route_leave(&self, pubkey: &str, channel_h: &str, now: u64) -> Result<()> {
        self.revoke_route_and_mark_absent(pubkey, channel_h, now)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "leave/tests.rs"]
mod tests;
