//! Durable exact-session channel affinity, independent of fabric standing.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmedAdmissionCommit {
    Committed,
    CleanupDue(SessionStanding),
    Superseded,
}

impl Store {
    pub fn commit_confirmed_session_admission(
        &self,
        pubkey: &str,
        channel_h: &str,
        runtime_generation: u64,
        lifecycle_epoch: u64,
        now: u64,
    ) -> Result<ConfirmedAdmissionCommit> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let owning_lifecycle = transaction
            .query_row(
                "SELECT lifecycle_epoch FROM sessions
                 WHERE pubkey=?1 AND runtime_generation=?2
                   AND recovery_state<>'revoked'",
                params![pubkey, runtime_generation],
                |row| row.get::<_, u64>(0),
            )
            .optional()?;
        let Some(owning_lifecycle) = owning_lifecycle else {
            let outcome = schedule_cleanup_in_transaction(
                &transaction,
                pubkey,
                channel_h,
                lifecycle_epoch,
                now,
            )?;
            transaction.commit()?;
            return self.finish_admission_outcome(pubkey, channel_h, outcome);
        };
        transaction.execute(
            "INSERT INTO session_channels
                 (pubkey, channel_h, joined_at, joined_event_seq)
             VALUES (?1, ?2, ?3,
                     (SELECT COALESCE(MAX(rowid), 0) FROM relay_events))
             ON CONFLICT(pubkey, channel_h) DO NOTHING",
            params![pubkey, channel_h, now],
        )?;
        transaction.execute(
            "INSERT INTO session_standing
                 (pubkey, channel_h, state, standing_epoch,
                  session_lifecycle_epoch, updated_at)
             VALUES (?1, ?2, 'member', 1, ?3, ?4)
             ON CONFLICT(pubkey, channel_h) DO UPDATE SET
                 state='member',
                 standing_epoch=session_standing.standing_epoch+1,
                 session_lifecycle_epoch=excluded.session_lifecycle_epoch,
                 updated_at=excluded.updated_at",
            params![pubkey, channel_h, owning_lifecycle, now],
        )?;
        transaction.commit()?;
        Ok(ConfirmedAdmissionCommit::Committed)
    }

    /// Persist compensation for relay admission whose primary commit failed.
    /// If the exact admission actually committed, or a newer lifecycle owns the
    /// member row, the result prevents a destructive stale removal.
    pub fn schedule_confirmed_admission_cleanup(
        &self,
        pubkey: &str,
        channel_h: &str,
        runtime_generation: u64,
        lifecycle_epoch: u64,
        now: u64,
    ) -> Result<ConfirmedAdmissionCommit> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let committed = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM sessions session
                 JOIN session_channels route ON route.pubkey=session.pubkey
                 JOIN session_standing standing ON standing.pubkey=session.pubkey
                 WHERE session.pubkey=?1 AND session.runtime_generation=?2
                   AND session.recovery_state<>'revoked'
                   AND route.channel_h=?3 AND standing.channel_h=?3
                   AND standing.state='member'
                   AND standing.session_lifecycle_epoch=session.lifecycle_epoch
             )",
            params![pubkey, runtime_generation, channel_h],
            |row| row.get::<_, bool>(0),
        )?;
        if committed {
            transaction.rollback()?;
            return Ok(ConfirmedAdmissionCommit::Committed);
        }
        let outcome =
            schedule_cleanup_in_transaction(&transaction, pubkey, channel_h, lifecycle_epoch, now)?;
        transaction.commit()?;
        self.finish_admission_outcome(pubkey, channel_h, outcome)
    }

    fn finish_admission_outcome(
        &self,
        pubkey: &str,
        channel_h: &str,
        outcome: PendingAdmissionOutcome,
    ) -> Result<ConfirmedAdmissionCommit> {
        match outcome {
            PendingAdmissionOutcome::CleanupDue => Ok(ConfirmedAdmissionCommit::CleanupDue(
                self.get_session_standing(pubkey, channel_h)?
                    .context("scheduled admission cleanup row disappeared")?,
            )),
            PendingAdmissionOutcome::Superseded => Ok(ConfirmedAdmissionCommit::Superseded),
        }
    }

    pub fn revoke_route_and_mark_absent(
        &self,
        pubkey: &str,
        channel_h: &str,
        now: u64,
    ) -> Result<bool> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let removed = transaction.execute(
            "DELETE FROM session_channels WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h],
        )? > 0;
        let updated = transaction.execute(
            "UPDATE session_standing
             SET state='absent',
                 standing_epoch=standing_epoch+1,
                 updated_at=?3
             WHERE pubkey=?1 AND channel_h=?2",
            params![pubkey, channel_h, now],
        )?;
        if updated == 0 {
            transaction.execute(
                "INSERT INTO session_standing
                 (pubkey, channel_h, state, standing_epoch,
                  session_lifecycle_epoch, updated_at)
             SELECT ?1, ?2, 'absent', 1, lifecycle_epoch, ?3
               FROM sessions WHERE pubkey=?1
             ON CONFLICT(pubkey, channel_h) DO NOTHING",
                params![pubkey, channel_h, now],
            )?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    pub fn grant_session_route(&self, pubkey: &str, channel_h: &str, joined_at: u64) -> Result<()> {
        if channel_h.trim().is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO session_channels
                 (pubkey, channel_h, joined_at, joined_event_seq)
             VALUES (?1, ?2, ?3,
                     (SELECT COALESCE(MAX(rowid), 0) FROM relay_events))",
            params![pubkey, channel_h, joined_at],
        )?;
        Ok(())
    }

    pub fn has_session_route(&self, pubkey: &str, channel_h: &str) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM session_channels WHERE pubkey=?1 AND channel_h=?2
             )",
            params![pubkey, channel_h],
            |row| row.get(0),
        )?)
    }

    /// Whether a stored event is eligible for automatic body delivery to this
    /// exact session membership.
    ///
    /// Both fences are required: local arrival order prevents a future-dated
    /// event observed before the join from replaying afterward, while signed
    /// time rejects backdated events first observed after the join. A missing
    /// event or membership row returns `false`.
    pub fn session_membership_admits_event(
        &self,
        pubkey: &str,
        channel_h: &str,
        event_id: &str,
    ) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1
                   FROM session_channels membership
                   JOIN relay_events event ON event.id=?3
                  WHERE membership.pubkey=?1
                    AND membership.channel_h=?2
                    AND event.channel_h=?2
                    AND event.rowid > membership.joined_event_seq
                    AND event.created_at >= membership.joined_at
             )",
            params![pubkey, channel_h, event_id],
            |row| row.get(0),
        )?)
    }

    pub fn list_session_routes(&self, pubkey: &str) -> Result<Vec<(String, u64)>> {
        let mut statement = self.conn.prepare(
            "SELECT channel_h, joined_at FROM session_channels
             WHERE pubkey=?1 ORDER BY joined_at, channel_h",
        )?;
        let rows = statement.query_map([pubkey], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAdmissionOutcome {
    CleanupDue,
    Superseded,
}

fn schedule_cleanup_in_transaction(
    transaction: &rusqlite::Transaction<'_>,
    pubkey: &str,
    channel_h: &str,
    lifecycle_epoch: u64,
    now: u64,
) -> Result<PendingAdmissionOutcome> {
    let newer_member = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM session_standing
         WHERE pubkey=?1 AND channel_h=?2 AND state='member'
           AND session_lifecycle_epoch<>?3)",
        params![pubkey, channel_h, lifecycle_epoch],
        |row| row.get::<_, bool>(0),
    )?;
    if newer_member {
        return Ok(PendingAdmissionOutcome::Superseded);
    }
    transaction.execute(
        "INSERT INTO session_standing
             (pubkey, channel_h, state, standing_epoch,
              session_lifecycle_epoch, updated_at)
         VALUES (?1, ?2, 'member', 1, ?3, ?4)
         ON CONFLICT(pubkey, channel_h) DO UPDATE SET
             state='member',
             standing_epoch=session_standing.standing_epoch+1,
             session_lifecycle_epoch=excluded.session_lifecycle_epoch,
             updated_at=excluded.updated_at",
        params![pubkey, channel_h, lifecycle_epoch, now],
    )?;
    Ok(PendingAdmissionOutcome::CleanupDue)
}

#[cfg(test)]
#[path = "session_routes/tests.rs"]
mod tests;
