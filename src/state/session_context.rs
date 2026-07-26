use super::*;

impl Store {
    pub fn set_session_readiness_parent(&self, pubkey: &str, readiness_parent: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET readiness_parent=?2 WHERE pubkey=?1",
            params![pubkey, readiness_parent],
        )?;
        Ok(())
    }

    pub fn session_readiness_parent(&self, channel_h: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT session.readiness_parent
                   FROM sessions session
                   JOIN session_channels membership
                     ON membership.pubkey=session.pubkey
                  WHERE membership.channel_h=?1
                    AND session.readiness_parent<>''
                  ORDER BY (session.runtime_state='running') DESC,
                           membership.joined_at DESC,
                           session.created_at DESC
                  LIMIT 1",
                [channel_h],
                |row| row.get::<_, String>(0),
            )
            .optional()?)
    }
}
