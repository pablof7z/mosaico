use super::*;

impl Store {
    pub(in crate::state) fn backfill_messages_from_relay_events(&self) -> Result<()> {
        self.conn.execute(
            "INSERT INTO messages
                 (message_id, thread_id, channel_h, author_pubkey, body,
                  created_at, direction, sync_state, native_event_id)
             SELECT id, channel_h, channel_h, pubkey, content, created_at,
                    'inbound', 'accepted', id
             FROM relay_events
             WHERE kind=9
             ON CONFLICT(message_id) DO NOTHING",
            [],
        )?;
        let cached_tags = {
            let mut stmt = self
                .conn
                .prepare("SELECT id, tags_json FROM relay_events WHERE kind=9")?;
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (message_id, tags_json) in cached_tags {
            for recipient in p_tag_pubkeys(&tags_json) {
                self.add_message_recipient(&message_id, &recipient, None)?;
            }
        }
        Ok(())
    }
}

fn p_tag_pubkeys(tags_json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Vec<String>>>(tags_json)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tag| {
            (tag.first().map(String::as_str) == Some("p"))
                .then(|| tag.get(1).cloned())
                .flatten()
        })
        .collect()
}
