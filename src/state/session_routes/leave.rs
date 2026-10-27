//! Explicit leave as a durable two-phase transition.
//!
//! Phase one removes the route but keeps the relay-confirmed `member`
//! standing. That pair is exactly the durable "relay removal is due" state
//! the standing cleanup already retries, and with the route gone presence no
//! longer projects the channel. Phase two records the confirmed relay
//! removal; a failed removal restores the original route.

use super::*;

/// The route a leave removed, kept so a failed relay removal can restore it
/// exactly (history cursor included).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeavingRoute {
    joined_at: u64,
    joined_event_seq: i64,
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
                "SELECT joined_at, joined_event_seq FROM session_channels
                 WHERE pubkey=?1 AND channel_h=?2",
                params![pubkey, channel_h],
                |row| {
                    Ok(LeavingRoute {
                        joined_at: row.get(0)?,
                        joined_event_seq: row.get(1)?,
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

    /// Restore the route a leave removed because the relay removal failed.
    pub fn restore_route_after_failed_leave(
        &self,
        pubkey: &str,
        channel_h: &str,
        route: &LeavingRoute,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO session_channels
                 (pubkey, channel_h, joined_at, joined_event_seq)
             VALUES (?1, ?2, ?3, ?4)",
            params![pubkey, channel_h, route.joined_at, route.joined_event_seq],
        )?;
        Ok(())
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
