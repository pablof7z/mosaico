use super::*;

/// Chat messages first observed after a cursor, in arrival order.
pub(crate) struct ArrivalBatch {
    pub(crate) messages: Vec<(u64, Message)>,
    /// The last arrival read, chat or not. Arrivals come only from the view, so
    /// one that is not a chat message there never becomes one, and a reader
    /// that matched nothing can move its cursor past every arrival scanned.
    pub(crate) scanned_through: u64,
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
    ) -> Result<ArrivalBatch> {
        let view = self.view();
        let limit = limit as usize;
        let mut rows = Vec::new();
        let mut scanned_through = after_sequence;
        if limit > 0 {
            self.visit_event_arrivals_after(after_sequence, |event_id, sequence| {
                scanned_through = sequence;
                if let Some(row) = view.message(event_id) {
                    rows.push((sequence, row.message.clone()));
                }
                rows.len() < limit
            })?;
        }
        let messages = rows
            .into_iter()
            .map(|(cursor, message)| Ok((cursor, self.with_local_attachment(message)?)))
            .collect::<Result<_>>()?;
        Ok(ArrivalBatch {
            messages,
            scanned_through,
        })
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
