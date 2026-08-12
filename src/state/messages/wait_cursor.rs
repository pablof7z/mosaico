use super::*;

impl Store {
    pub(crate) fn latest_message_arrival_sequence(&self) -> Result<u64> {
        self.latest_nmp_arrival_sequence()
    }

    pub(crate) fn message_arrival_sequence(&self, message_id: &str) -> Result<Option<u64>> {
        self.nmp_arrival_sequence(message_id)
    }

    pub(crate) fn messages_after_arrival_sequence(
        &self,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<(u64, Message)>> {
        let mut rows = Vec::new();
        for row in self.nmp_views.messages() {
            let Some(sequence) = self.nmp_arrival_sequence(&row.message.message_id)? else {
                continue;
            };
            rows.push((sequence, row.message));
        }
        rows.retain(|(sequence, _)| *sequence > after_sequence);
        rows.sort_by_key(|(cursor, _)| *cursor);
        rows.truncate(limit as usize);
        rows.into_iter()
            .map(|(cursor, message)| Ok((cursor, self.with_local_attachment(message)?)))
            .collect()
    }

    pub(crate) fn message_reply_target(&self, message: &Message) -> Result<Option<String>> {
        Ok(self
            .nmp_views
            .message(&message.message_id)
            .and_then(|row| row.reply_target))
    }
}

#[cfg(test)]
#[path = "wait_cursor/tests.rs"]
mod tests;
