use super::*;

impl Store {
    /// Persist the caller-declared broad title. `work_topic` is kept as the
    /// distillation-suppression marker; the public session title changes now.
    pub fn set_session_work_topic(&self, pubkey: &str, topic: &str, set_at: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET work_topic=?2, work_topic_set_at=?3, title=?2 WHERE pubkey=?1",
            params![pubkey, topic, set_at],
        )?;
        Ok(())
    }
}
