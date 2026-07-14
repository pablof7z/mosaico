//! Per-session chat publication markers.

use super::*;

impl Store {
    /// Mark that this session has successfully published through an explicit
    /// channel command. Keeps the earliest non-zero marker stable.
    pub fn mark_session_explicit_chat_published(&self, pubkey: &str, at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions
             SET explicit_chat_published_at=CASE
                 WHEN explicit_chat_published_at=0 THEN ?2
                 ELSE explicit_chat_published_at
             END
             WHERE pubkey=?1",
            params![pubkey, at],
        )?;
        Ok(())
    }
}
