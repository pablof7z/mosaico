use super::*;

impl Store {
    pub(crate) fn latest_message_arrival_sequence(&self) -> Result<u64> {
        self.latest_event_arrival_sequence()
    }

    pub(crate) fn message_arrival_sequence(&self, message_id: &str) -> Result<Option<u64>> {
        self.event_arrival_sequence(message_id)
    }

    pub(crate) fn messages_after_arrival_sequence(
        &self,
        after_sequence: u64,
        limit: u32,
    ) -> Result<Vec<(u64, Message)>> {
        let view = self.view();
        let mut rows = Vec::new();
        for (event_id, sequence) in self.event_arrivals_after(after_sequence)? {
            if rows.len() == limit as usize {
                break;
            }
            // Arrivals that are not chat messages in the current view drop out.
            if let Some(row) = view.message(&event_id) {
                rows.push((sequence, row.message.clone()));
            }
        }
        rows.into_iter()
            .map(|(cursor, message)| Ok((cursor, self.with_local_attachment(message)?)))
            .collect()
    }

    pub(crate) fn message_reply_target(&self, message: &Message) -> Result<Option<String>> {
        Ok(self
            .view()
            .message(&message.message_id)
            .and_then(|row| row.reply_target.clone()))
    }
}

#[cfg(test)]
#[path = "wait_cursor/tests.rs"]
mod tests;
