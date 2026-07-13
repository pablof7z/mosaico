//! `messages` / `message_recipients` — canonical channel read model.
//!
//! `relay_events` remains the verbatim wire cache. These rows are the stable
//! reader-facing shape: message body, author return envelope, sync state, and
//! recipient edges. Delivery state still lives in `inbox`.

use super::*;

mod wait_cursor;

const MESSAGE_COLS: &str = "message_id, thread_id, channel_h, author_pubkey, author_session, \
     body, created_at, direction, sync_state, native_event_id, error";
const RECIPIENT_COLS: &str = "message_id, recipient_pubkey, target_session, delivered_at";

fn opt_text(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

fn row_to_message(row: &rusqlite::Row) -> rusqlite::Result<Message> {
    Ok(Message {
        message_id: row.get(0)?,
        thread_id: row.get(1)?,
        channel_h: row.get(2)?,
        author_pubkey: row.get(3)?,
        author_session: opt_text(row.get(4)?),
        body: row.get(5)?,
        created_at: row.get(6)?,
        direction: row.get(7)?,
        sync_state: row.get(8)?,
        native_event_id: opt_text(row.get(9)?),
        error: opt_text(row.get(10)?),
    })
}

fn row_to_recipient(row: &rusqlite::Row) -> rusqlite::Result<MessageRecipient> {
    let delivered_at: u64 = row.get(3)?;
    Ok(MessageRecipient {
        message_id: row.get(0)?,
        recipient_pubkey: row.get(1)?,
        target_session: opt_text(row.get(2)?),
        delivered_at: (delivered_at > 0).then_some(delivered_at),
    })
}

impl Store {
    pub(super) fn backfill_messages_from_relay_events(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages
                 (message_id, thread_id, channel_h, author_pubkey, author_session, body,
                  created_at, direction, sync_state, native_event_id)
             SELECT
                 id,
                 channel_h,
                 channel_h,
                 pubkey,
                 (
                     SELECT NULLIF(session_id, '') FROM relay_status
                     WHERE relay_status.pubkey=relay_events.pubkey
                       AND relay_status.channel_h=relay_events.channel_h
                     ORDER BY updated_at DESC LIMIT 1
                 ),
                 content,
                 created_at,
                 'inbound',
                 'accepted',
                 id
             FROM relay_events
             WHERE kind=9
             ON CONFLICT(message_id) DO NOTHING",
            [],
        )?;
        Ok(())
    }

    /// Record or refresh one canonical message row. Idempotent by `message_id`:
    /// local optimistic writes and relay replay can both materialize the same
    /// event without dropping a previously-known sender session.
    pub fn record_message(&self, msg: &RecordMessage) -> Result<String> {
        let message_id = msg.message_id.trim();
        if message_id.is_empty() {
            anyhow::bail!("message_id must not be empty");
        }
        let author_session = msg.author_session.as_deref().unwrap_or("");
        let native_event_id = msg.native_event_id.as_deref().unwrap_or("");
        let error = msg.error.as_deref().unwrap_or("");
        self.conn.execute(
            "INSERT INTO messages
                 (message_id, thread_id, channel_h, author_pubkey, author_session, body,
                  created_at, direction, sync_state, native_event_id, error)
             VALUES (?1, ?2, ?3, ?4, NULLIF(?5, ''), ?6, ?7, ?8, ?9, NULLIF(?10, ''), NULLIF(?11, ''))
             ON CONFLICT(message_id) DO UPDATE SET
                 thread_id=excluded.thread_id,
                 channel_h=excluded.channel_h,
                 author_pubkey=excluded.author_pubkey,
                 author_session=COALESCE(excluded.author_session, messages.author_session),
                 body=excluded.body,
                 created_at=excluded.created_at,
                 direction=CASE
                     WHEN messages.direction='outbound' THEN messages.direction
                     ELSE excluded.direction
                 END,
                 sync_state=excluded.sync_state,
                 native_event_id=COALESCE(excluded.native_event_id, messages.native_event_id),
                 error=excluded.error",
            params![
                message_id,
                msg.thread_id,
                msg.channel_h,
                msg.author_pubkey,
                author_session,
                msg.body,
                msg.created_at,
                msg.direction,
                msg.sync_state,
                native_event_id,
                error
            ],
        )?;
        Ok(message_id.to_string())
    }

    /// Record a recipient edge for a message. `target_session` is canonicalized
    /// when possible; empty target means the fabric only supplied recipient pubkey.
    pub fn add_message_recipient(
        &self,
        message_id: &str,
        recipient_pubkey: &str,
        target_session: Option<&str>,
        delivered_at: Option<u64>,
    ) -> Result<()> {
        let target = match target_session.filter(|s| !s.is_empty()) {
            Some(raw) => self
                .resolve_canonical_id(raw)?
                .unwrap_or_else(|| raw.to_string()),
            None => String::new(),
        };
        let delivered_at = delivered_at.unwrap_or(0);
        self.conn.execute(
            "INSERT INTO message_recipients
                 (message_id, recipient_pubkey, target_session, delivered_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(message_id, recipient_pubkey, target_session) DO UPDATE SET
                 delivered_at=MAX(message_recipients.delivered_at, excluded.delivered_at)",
            params![message_id, recipient_pubkey, target, delivered_at],
        )?;
        Ok(())
    }

    pub fn get_message_by_prefix(&self, prefix: &str) -> Result<Option<Message>> {
        if prefix.len() >= 64 {
            return self.get_message(prefix);
        }
        let pattern = format!("{prefix}*");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM messages
             WHERE message_id GLOB ?1 OR native_event_id GLOB ?1
             ORDER BY created_at DESC, message_id DESC LIMIT 2"
        ))?;
        let mut rows = stmt.query_map(params![pattern], row_to_message)?;
        let first = rows.next().transpose()?;
        if rows.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one message");
        }
        Ok(first)
    }

    pub fn get_message(&self, message_id: &str) -> Result<Option<Message>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {MESSAGE_COLS} FROM messages WHERE message_id=?1"),
                params![message_id],
                row_to_message,
            )
            .optional()?)
    }

    pub fn chat_messages_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM messages
             WHERE channel_h=?1 AND created_at > ?2
             ORDER BY created_at ASC, message_id ASC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![channel_h, since, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn chat_messages_for_channel_after(
        &self,
        channel_h: &str,
        after_created_at: u64,
        after_id: &str,
        limit: u32,
    ) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM messages
             WHERE channel_h=?1
               AND (created_at > ?2 OR (created_at = ?2 AND message_id > ?3))
             ORDER BY created_at ASC, message_id ASC LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![channel_h, after_created_at, after_id, limit],
            row_to_message,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn recent_chat_messages(&self, since: u64, limit: u32) -> Result<Vec<Message>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {MESSAGE_COLS} FROM messages
             WHERE created_at >= ?1
             ORDER BY created_at DESC, message_id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![since, limit], row_to_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn session_has_outbound_message_since(&self, session_id: &str, since: u64) -> Result<bool> {
        let Some(canonical) = self.resolve_canonical_id(session_id)? else {
            return Ok(false);
        };
        let exists: i64 = self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM messages
                 WHERE author_session=?1
                   AND direction='outbound'
                   AND sync_state='accepted'
                   AND created_at >= ?2
             )",
            params![canonical, since],
            |row| row.get(0),
        )?;
        Ok(exists != 0)
    }

    pub fn message_recipients(&self, message_id: &str) -> Result<Vec<MessageRecipient>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {RECIPIENT_COLS} FROM message_recipients
             WHERE message_id=?1 ORDER BY recipient_pubkey ASC, target_session ASC"
        ))?;
        let rows = stmt.query_map(params![message_id], row_to_recipient)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests;
