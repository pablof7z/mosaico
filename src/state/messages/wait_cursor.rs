use super::*;

fn row_to_message_with_rowid(row: &rusqlite::Row) -> rusqlite::Result<(i64, Message)> {
    Ok((
        row.get(0)?,
        Message {
            message_id: row.get(1)?,
            thread_id: row.get(2)?,
            channel_h: row.get(3)?,
            author_pubkey: row.get(4)?,
            body: row.get(5)?,
            created_at: row.get(6)?,
            direction: row.get(7)?,
            sync_state: row.get(8)?,
            native_event_id: opt_text(row.get(9)?),
            error: opt_text(row.get(10)?),
        },
    ))
}

fn reply_target_from_tags_json(tags_json: &str) -> Option<String> {
    let tags: Vec<Vec<String>> = serde_json::from_str(tags_json).ok()?;
    let event_tags = tags
        .iter()
        .filter(|tag| tag.first().map(String::as_str) == Some("e"))
        .filter_map(|tag| tag.get(1).filter(|id| !id.is_empty()).map(|id| (tag, id)));
    let mut fallback = None;
    for (tag, id) in event_tags {
        if tag.get(3).map(String::as_str) == Some("reply") {
            return Some(id.clone());
        }
        fallback = Some(id.clone());
    }
    fallback
}

impl Store {
    /// SQLite rowid is the daemon-local arrival cursor for one blocking wait.
    /// It is never exposed as fabric identity and need not survive a rebuild.
    pub(crate) fn latest_message_rowid(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(MAX(rowid), 0) FROM messages", [], |row| {
                row.get(0)
            })?)
    }

    pub(crate) fn message_rowid(&self, message_id: &str) -> Result<Option<i64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT rowid FROM messages WHERE message_id=?1",
                [message_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn messages_after_rowid(
        &self,
        after_rowid: i64,
        limit: u32,
    ) -> Result<Vec<(i64, Message)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT rowid, {MESSAGE_COLS} FROM messages
             WHERE rowid > ?1 ORDER BY rowid ASC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![after_rowid, limit], row_to_message_with_rowid)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Immediate reply target from the native kind:9 `e` tags. Prefer the
    /// NIP-10 `reply` marker; for the canonical bare single-tag form (and old
    /// positional NIP-10) the last `e` tag is the immediate parent.
    pub(crate) fn message_reply_target(&self, message: &Message) -> Result<Option<String>> {
        let event_id = message
            .native_event_id
            .as_deref()
            .unwrap_or(&message.message_id);
        Ok(self
            .get_event(event_id)?
            .and_then(|event| reply_target_from_tags_json(&event.tags_json)))
    }
}

#[cfg(test)]
#[path = "wait_cursor/tests.rs"]
mod tests;
