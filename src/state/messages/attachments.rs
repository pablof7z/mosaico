use super::*;
use std::path::Path;

impl Store {
    /// Attach a successfully materialized local directory without allowing a
    /// later relay replay or retry to erase or replace it.
    pub fn set_message_attachment_dir(&self, message_id: &str, directory: &Path) -> Result<bool> {
        let directory = directory
            .to_str()
            .context("attachment directory path is not valid UTF-8")?;
        let changed = self.conn.execute(
            "UPDATE messages SET attachment_dir=?2
             WHERE message_id=?1 AND attachment_dir=''",
            params![message_id, directory],
        )?;
        Ok(changed > 0)
    }
}
