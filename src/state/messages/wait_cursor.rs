use super::*;

/// A chat message with its arrival sequence and the reply target from the
/// same view level, so a reader never needs a second lookup that could see a
/// newer level.
pub(crate) struct ArrivedMessage {
    pub(crate) sequence: u64,
    pub(crate) message: Message,
    pub(crate) reply_target: Option<String>,
}

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
    ) -> Result<Vec<ArrivedMessage>> {
        let view = self.view();
        let limit = limit as usize;
        let mut rows = Vec::new();
        if limit > 0 {
            self.visit_event_arrivals_after(after_sequence, |event_id, sequence| {
                // Arrivals that are not chat messages in this level are skipped,
                // never consumed: a chat event can be briefly absent while a
                // demand is replaced, and must still be read once it returns.
                if let Some(row) = view.message(event_id) {
                    rows.push((sequence, row));
                }
                rows.len() < limit
            })?;
        }
        rows.into_iter()
            .map(|(sequence, row)| {
                Ok(ArrivedMessage {
                    sequence,
                    message: self.with_local_attachment(row.message.clone())?,
                    reply_target: row.reply_target.clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "wait_cursor/tests.rs"]
mod tests;
