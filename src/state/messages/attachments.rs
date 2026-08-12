use super::*;
use std::path::Path;

impl Store {
    /// Keep the first successfully materialized local directory for an event.
    /// This is intentionally independent of whether NMP currently observes it.
    pub fn set_message_attachment_dir(&self, event_id: &str, directory: &Path) -> Result<bool> {
        let directory = directory
            .to_str()
            .context("attachment directory path is not valid UTF-8")?;
        let changed = self.conn.execute(
            "INSERT INTO message_attachments (event_id, directory)
             VALUES (?1, ?2)
             ON CONFLICT(event_id) DO NOTHING",
            params![event_id, directory],
        )?;
        Ok(changed > 0)
    }

    pub(super) fn message_attachment_dir(&self, event_id: &str) -> Result<String> {
        Ok(self
            .conn
            .query_row(
                "SELECT directory FROM message_attachments WHERE event_id=?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or_default())
    }
}
