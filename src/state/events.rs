//! Raw event reads projected from Fava's current delivered rows.

use super::*;

impl Store {
    pub fn get_event(&self, id: &str) -> Result<Option<RelayEvent>> {
        Ok(self.observed_views.recent_rows().event(id))
    }

    pub fn get_event_by_prefix(&self, prefix: &str) -> Result<Option<RelayEvent>> {
        self.observed_views.recent_rows().event_by_prefix(prefix)
    }

    pub fn has_event(&self, id: &str) -> Result<bool> {
        Ok(self.observed_views.recent_rows().event(id).is_some())
    }

    pub fn chat_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<RelayEvent>> {
        Ok(self
            .observed_views
            .recent_rows()
            .events_in_channel(channel_h, since, limit))
    }

    pub fn latest_message_at_by_pubkey(
        &self,
        channel_h: &str,
    ) -> Result<std::collections::HashMap<String, u64>> {
        Ok(self
            .observed_views
            .recent_rows()
            .latest_chat_by_author(channel_h, crate::fabric::nip29::wire::KIND_CHAT)
            .into_iter()
            .collect())
    }

    pub fn prejoin_chat_for_session(
        &self,
        pubkey: &str,
        channel_h: &str,
        limit: u32,
    ) -> Result<Vec<RelayEvent>> {
        let joined_sequence = self
            .conn
            .query_row(
                "SELECT joined_event_seq FROM session_channels
                 WHERE pubkey=?1 AND channel_h=?2",
                params![pubkey, channel_h],
                |row| row.get::<_, u64>(0),
            )
            .optional()?;
        let Some(joined_sequence) = joined_sequence else {
            return Ok(Vec::new());
        };
        let mut events = Vec::new();
        for event in self
            .observed_views
            .recent_rows()
            .events_in_channel(channel_h, 0, u32::MAX)
        {
            if event.kind != crate::fabric::nip29::wire::KIND_CHAT as u32 {
                continue;
            }
            if self
                .event_arrival_sequence(&event.id)?
                .is_some_and(|sequence| sequence <= joined_sequence)
            {
                events.push(event);
            }
        }
        events.sort_by(|left, right| {
            (&right.created_at, &right.id).cmp(&(&left.created_at, &left.id))
        });
        events.truncate(limit as usize);
        Ok(events)
    }

    pub fn events_by_kind(&self, kind: u32, limit: u32) -> Result<Vec<RelayEvent>> {
        let kind = u16::try_from(kind).context("Nostr kind exceeds u16")?;
        Ok(self
            .observed_views
            .recent_rows()
            .events_by_kind(kind, limit))
    }
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
