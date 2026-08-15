use super::{row_to_inbox, COLS};
use crate::state::*;

impl Store {
    /// Atomically claim every pending row for an agent: flip each to
    /// `delivered` AND return it in a single statement. The FIRST caller - the
    /// direct injection path or a hook - wins; any concurrent caller gets an
    /// empty vec. This atomicity IS the dedup: a message can only be injected
    /// once, with no separate "notified" flag or external gate. Rows come back
    /// oldest-first (RETURNING order is unspecified, so we sort).
    pub fn claim_pending_for_pubkey(&self, target_pubkey: &str, now: u64) -> Result<Vec<InboxRow>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let event_ids = {
            let mut stmt = transaction.prepare(
                "SELECT event_id FROM inbox
                 WHERE target_pubkey=?1 AND state='pending'",
            )?;
            let rows = stmt.query_map([target_pubkey], |row| row.get::<_, String>(0))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut out = Vec::new();
        for event_id in event_ids {
            let changed = transaction.execute(
                "UPDATE inbox SET state='delivered', delivered_at=?3
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
            let rows = stmt.query_map(params![event_id, target_pubkey], row_to_inbox)?;
            out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        out.sort_by_key(|r| r.created_at);
        crate::state::work_start::stage_from_inbox_tx(&transaction, &out, now)?;
        transaction.commit()?;
        Ok(out)
    }

    /// Atomically claim only the specified pending event ids for an agent.
    /// The delivery reconciler plans against exact inbox ids; this applies that
    /// plan without consuming rows that arrived after the scan.
    pub fn claim_pending_event_ids_for_pubkey(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        now: u64,
    ) -> Result<Vec<InboxRow>> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut out = Vec::new();
        for id in event_ids {
            let changed = transaction.execute(
                "UPDATE inbox SET state='delivered', delivered_at=?3
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='pending'",
                params![id, target_pubkey, now],
            )?;
            if changed > 0 {
                let mut stmt = transaction.prepare(&format!(
                    "SELECT {COLS} FROM inbox
                     LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
                     WHERE inbox.event_id=?1 AND inbox.target_pubkey=?2"
                ))?;
                let rows = stmt.query_map(params![id, target_pubkey], row_to_inbox)?;
                out.extend(rows.collect::<rusqlite::Result<Vec<_>>>()?);
            }
        }
        out.sort_by_key(|r| r.created_at);
        transaction.commit()?;
        Ok(out)
    }

    /// Roll claimed rows back to `pending` so they are retried rather than lost.
    /// Used only when direct injection fails AFTER the atomic claim.
    pub fn reenqueue_pending(&self, event_ids: &[String], target_pubkey: &str) -> Result<()> {
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='pending', delivered_at=0
                 WHERE event_id=?1 AND target_pubkey=?2",
                params![id, target_pubkey],
            )?;
        }
        Ok(())
    }

    /// Completed inbound rows for an agent whose delivery is newer than
    /// `since`, oldest-first. Powers integration peeks.
    pub fn recently_delivered_for_pubkey(
        &self,
        target_pubkey: &str,
        since: u64,
    ) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
             WHERE inbox.target_pubkey=?1
               AND inbox.state IN ('delivered', 'submitted', 'injected', 'echo_consumed')
               AND inbox.delivered_at>=?2
             ORDER BY inbox.created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey, since], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// PTY wrote the mention as a user prompt; wait for the harness
    /// user-prompt-submit hook before treating delivery as confirmed.
    pub fn mark_submitted_for_prompt_confirm(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        now: u64,
    ) -> Result<()> {
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='submitted', delivered_at=?3
                 WHERE event_id=?1 AND target_pubkey=?2
                   AND state IN ('delivered', 'pending')",
                params![id, target_pubkey, now],
            )?;
        }
        Ok(())
    }

    /// Mark rows as confirmed PTY user-prompt input (echo-suppress in fabric
    /// context). Stages work-start handoffs for the confirmed boundary.
    pub fn mark_injected_for_echo(
        &self,
        event_ids: &[String],
        target_pubkey: &str,
        now: u64,
    ) -> Result<()> {
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.conn,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        for id in event_ids {
            transaction.execute(
                "UPDATE inbox SET state='injected'
                 WHERE event_id=?1 AND target_pubkey=?2
                   AND state IN ('delivered', 'submitted')",
                params![id, target_pubkey],
            )?;
        }
        let mut rows = Vec::new();
        for id in event_ids {
            let mut stmt = transaction.prepare(&format!(
                "SELECT {COLS} FROM inbox
                 LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
                 WHERE inbox.event_id=?1 AND inbox.target_pubkey=?2
                   AND inbox.state='injected'"
            ))?;
            let claimed = stmt.query_map(params![id, target_pubkey], row_to_inbox)?;
            rows.extend(claimed.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        crate::state::work_start::stage_from_inbox_tx(&transaction, &rows, now)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn submitted_for_pubkey(&self, target_pubkey: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
             WHERE inbox.target_pubkey=?1 AND inbox.state='submitted'
             ORDER BY inbox.delivered_at ASC, inbox.created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn injected_for_pubkey(&self, target_pubkey: &str) -> Result<Vec<InboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM inbox
             LEFT JOIN message_attachments ON message_attachments.event_id=inbox.event_id
             WHERE inbox.target_pubkey=?1 AND inbox.state='injected'
             ORDER BY inbox.delivered_at ASC, inbox.created_at ASC"
        ))?;
        let rows = stmt.query_map(params![target_pubkey], row_to_inbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Promote `submitted` rows whose terminal envelope appears in the harness
    /// user prompt. Returns confirmed event ids. Remaining submissions are left
    /// for [`Self::reenqueue_submitted`].
    pub fn confirm_submitted_from_prompt(
        &self,
        target_pubkey: &str,
        prompt: &str,
        now: u64,
    ) -> Result<Vec<String>> {
        let submitted = self.submitted_for_pubkey(target_pubkey)?;
        let confirmed: Vec<String> = submitted
            .into_iter()
            .filter(|row| prompt_corroborates_submission(prompt, row))
            .map(|row| row.event_id)
            .collect();
        if !confirmed.is_empty() {
            self.mark_injected_for_echo(&confirmed, target_pubkey, now)?;
        }
        Ok(confirmed)
    }

    /// Roll unconfirmed PTY submissions back to `pending` so hook delivery or a
    /// later inject can retry. Returns the re-enqueued event ids.
    pub fn reenqueue_submitted(&self, target_pubkey: &str) -> Result<Vec<String>> {
        let submitted = self.submitted_for_pubkey(target_pubkey)?;
        let ids: Vec<String> = submitted.into_iter().map(|row| row.event_id).collect();
        if !ids.is_empty() {
            self.reenqueue_pending(&ids, target_pubkey)?;
        }
        Ok(ids)
    }

    /// Re-enqueue `submitted` rows whose write is older than `before` (unix secs).
    pub fn reenqueue_stale_submitted(
        &self,
        target_pubkey: &str,
        before: u64,
    ) -> Result<Vec<String>> {
        let submitted = self.submitted_for_pubkey(target_pubkey)?;
        let ids: Vec<String> = submitted
            .into_iter()
            .filter(|row| row.delivered_at > 0 && row.delivered_at < before)
            .map(|row| row.event_id)
            .collect();
        if !ids.is_empty() {
            self.reenqueue_pending(&ids, target_pubkey)?;
        }
        Ok(ids)
    }

    pub fn consume_injected_echo(&self, event_ids: &[String], target_pubkey: &str) -> Result<()> {
        for id in event_ids {
            self.conn.execute(
                "UPDATE inbox SET state='echo_consumed'
                 WHERE event_id=?1 AND target_pubkey=?2 AND state='injected'",
                params![id, target_pubkey],
            )?;
        }
        Ok(())
    }
}

fn prompt_corroborates_submission(prompt: &str, row: &InboxRow) -> bool {
    if prompt.is_empty() {
        return false;
    }
    let short = crate::util::short_id(&row.event_id);
    // Terminal inject envelopes carry `id="<short>"` (see agent_xml::write_message).
    if !short.is_empty() && prompt.contains(&format!("id=\"{short}\"")) {
        return true;
    }
    if prompt.contains(&row.event_id) {
        return true;
    }
    // Body fallback when the harness strips attributes but keeps content.
    let body = row.body.trim();
    !body.is_empty() && body.len() >= 12 && prompt.contains(body)
}
