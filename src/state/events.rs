//! Raw event reads projected from current Nostr runtime subscription levels.

use super::*;

impl Store {
    pub fn get_event(&self, id: &str) -> Result<Option<RelayEvent>> {
        Ok(self.view().event(id).cloned())
    }

    pub fn get_event_by_prefix(&self, prefix: &str) -> Result<Option<RelayEvent>> {
        let view = self.view();
        let mut matches = view
            .events()
            .iter()
            .filter(|event| event.id.starts_with(prefix));
        let first = matches.next().cloned();
        if matches.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one event");
        }
        Ok(first)
    }

    pub fn has_event(&self, id: &str) -> Result<bool> {
        Ok(self.view().event(id).is_some())
    }

    pub fn chat_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<RelayEvent>> {
        let mut events = self
            .view()
            .events()
            .iter()
            .filter(|event| event.channel_h == channel_h && event.created_at >= since)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            (&left.created_at, &left.id).cmp(&(&right.created_at, &right.id))
        });
        events.truncate(limit as usize);
        Ok(events)
    }

    pub fn latest_message_at_by_pubkey(
        &self,
        channel_h: &str,
    ) -> Result<std::collections::HashMap<String, u64>> {
        Ok(self
            .view()
            .events()
            .iter()
            .filter(|event| {
                event.channel_h == channel_h
                    && event.kind == crate::fabric::nip29::wire::KIND_CHAT as u32
            })
            .fold(std::collections::HashMap::new(), |mut latest, event| {
                latest
                    .entry(event.pubkey.clone())
                    .and_modify(|current| *current = (*current).max(event.created_at))
                    .or_insert(event.created_at);
                latest
            }))
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
            .view()
            .events()
            .iter()
            .filter(|event| event.channel_h == channel_h)
        {
            if event.kind != crate::fabric::nip29::wire::KIND_CHAT as u32 {
                continue;
            }
            if self
                .event_arrival_sequence(&event.id)?
                .is_some_and(|sequence| sequence <= joined_sequence)
            {
                events.push(event.clone());
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
        let mut events = self
            .view()
            .events()
            .iter()
            .filter(|event| event.kind == kind as u32)
            .cloned()
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            (&right.created_at, &right.id).cmp(&(&left.created_at, &left.id))
        });
        events.truncate(limit as usize);
        Ok(events)
    }
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
