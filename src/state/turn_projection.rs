use super::*;

impl Store {
    /// Apply the Trellis-derived local turn projection. The graph decides the
    /// values; this method only writes them to the canonical row.
    pub fn apply_turn_projection(
        &self,
        pubkey: &str,
        working: bool,
        turn_started_at: u64,
        transcript_ref: Option<&str>,
    ) -> Result<()> {
        if let Some(transcript) = transcript_ref {
            self.conn.execute(
                "UPDATE sessions
                 SET working=?2, turn_started_at=?3, transcript_path=?4
                 WHERE pubkey=?1",
                params![pubkey, working as i64, turn_started_at, transcript],
            )?;
        } else {
            self.conn.execute(
                "UPDATE sessions
                 SET working=?2, turn_started_at=?3
                 WHERE pubkey=?1",
                params![pubkey, working as i64, turn_started_at],
            )?;
        }
        Ok(())
    }

    /// Apply the Trellis-derived cursor transition. The cursor graph decides
    /// whether a render request advances; this method only writes the result.
    pub fn apply_cursor_projection(&self, pubkey: &str, seen_cursor: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET seen_cursor=?2 WHERE pubkey=?1",
            params![pubkey, seen_cursor],
        )?;
        Ok(())
    }
}
