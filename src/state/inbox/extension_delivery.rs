use super::{row_to_inbox, COLS};
use crate::state::*;

impl Store {
    /// Lease every pending row to an extension-owned delivery channel. Unlike
    /// transport claims, a lease does not stage work or mark the row injected:
    /// the extension must later acknowledge Pi's matching custom message.
    pub fn lease_pending_for_extension(
        &self,
        target_pubkey: &str,
        now: u64,
    ) -> Result<Vec<InboxRow>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let event_ids = {
            let mut stmt = transaction.prepare(
                "SELECT event_id FROM inbox
                 WHERE target_pubkey=?1 AND state='pending'",
            )?;
            let ids = stmt
                .query_map([target_pubkey], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        let mut rows = Vec::new();
        for event_id in event_ids {
            let changed = transaction.execute(
                "UPDATE inbox SET state='leased', delivered_at=?3
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='pending'",
                params![event_id, target_pubkey, now],
            )?;
            if changed == 0 {
                continue;
            }
            let mut stmt = transaction.prepare(&format!(
                "SELECT {COLS} FROM inbox
                 LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
                 WHERE inbox.event_id=?1 AND inbox.target_pubkey=?2"
            ))?;
            rows.extend(
                stmt.query_map(params![event_id, target_pubkey], row_to_inbox)?
                    .collect::<rusqlite::Result<Vec<_>>>()?,
            );
        }
        rows.sort_by_key(|row| row.created_at);
        transaction.commit()?;
        Ok(rows)
    }

    /// Finalize (or decline) exactly the rows previously leased to an extension.
    /// A caller supplies the ids from its in-memory lease token; rows from any
    /// later delivery attempt remain untouched.
    pub fn acknowledge_extension_lease(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        accepted: bool,
        now: u64,
    ) -> Result<Vec<String>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut confirmed = Vec::new();
        let mut injected = Vec::new();
        for event_id in event_ids {
            let state = if accepted { "injected" } else { "pending" };
            let delivered_at = if accepted { now } else { 0 };
            let changed = transaction.execute(
                "UPDATE inbox SET state=?3, delivered_at=?4
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='leased'",
                params![event_id, target_pubkey, state, delivered_at],
            )?;
            if changed == 0 {
                continue;
            }
            confirmed.push(event_id.clone());
            if accepted {
                let mut stmt = transaction.prepare(&format!(
                    "SELECT {COLS} FROM inbox
                     LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
                     WHERE inbox.event_id=?1 AND inbox.target_pubkey=?2"
                ))?;
                injected.extend(
                    stmt.query_map(params![event_id, target_pubkey], row_to_inbox)?
                        .collect::<rusqlite::Result<Vec<_>>>()?,
                );
            }
        }
        if accepted && !injected.is_empty() {
            crate::state::work_start::stage_from_inbox_tx(&transaction, &injected, now)?;
        }
        transaction.commit()?;
        Ok(confirmed)
    }

    /// Recover extension claims that could not survive a daemon restart or a
    /// dropped Pi delivery channel. These rows were never confirmed injected.
    pub fn reenqueue_extension_leases(&self, target_pubkey: Option<&str>) -> Result<Vec<String>> {
        let mut query = String::from("SELECT event_id FROM inbox WHERE state='leased'");
        if target_pubkey.is_some() {
            query.push_str(" AND target_pubkey=?1");
        }
        let mut stmt = self.conn.prepare(&query)?;
        let ids = match target_pubkey {
            Some(pubkey) => stmt
                .query_map([pubkey], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?,
        };
        if ids.is_empty() {
            return Ok(ids);
        }
        let changed = match target_pubkey {
            Some(pubkey) => self.conn.execute(
                "UPDATE inbox SET state='pending', delivered_at=0
                 WHERE target_pubkey=?1 AND state='leased'",
                [pubkey],
            )?,
            None => self.conn.execute(
                "UPDATE inbox SET state='pending', delivered_at=0 WHERE state='leased'",
                [],
            )?,
        };
        debug_assert_eq!(changed, ids.len());
        Ok(ids)
    }

    /// Requeue only the named lease rows. An expired token must never roll
    /// back a different, newer lease for the same recipient.
    pub fn reenqueue_extension_lease_ids(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
    ) -> Result<Vec<String>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut requeued = Vec::new();
        for event_id in event_ids {
            let changed = transaction.execute(
                "UPDATE inbox SET state='pending', delivered_at=0
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='leased'",
                params![event_id, target_pubkey],
            )?;
            if changed > 0 {
                requeued.push(event_id.clone());
            }
        }
        transaction.commit()?;
        Ok(requeued)
    }
}
