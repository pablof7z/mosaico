use std::collections::HashMap;

use crate::fabric::nip29::wire::KIND_CHAT;
use crate::state::RelayEvent;

use super::NmpViews;

impl NmpViews {
    pub(crate) fn event(&self, id: &str) -> Option<RelayEvent> {
        self.projected_event(id).map(|row| row.event)
    }

    pub(crate) fn event_by_prefix(&self, prefix: &str) -> anyhow::Result<Option<RelayEvent>> {
        if prefix.len() >= 64 {
            return Ok(self.event(prefix));
        }
        let mut matches = self
            .projected_events()
            .into_iter()
            .filter(|row| row.event.id.starts_with(prefix))
            .map(|row| row.event);
        let first = matches.next();
        if matches.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one event");
        }
        Ok(first)
    }

    pub(crate) fn events_in_channel(
        &self,
        channel: &str,
        since: u64,
        limit: u32,
    ) -> Vec<RelayEvent> {
        let mut events = self
            .projected_events()
            .into_iter()
            .map(|row| row.event)
            .filter(|event| event.channel_h == channel && event.created_at > since)
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            (&left.created_at, &left.id).cmp(&(&right.created_at, &right.id))
        });
        events.truncate(limit as usize);
        events
    }

    pub(crate) fn events_in_channel_after(
        &self,
        channel: &str,
        after_created_at: u64,
        after_id: &str,
        limit: u32,
    ) -> Vec<RelayEvent> {
        let mut events = self
            .projected_events()
            .into_iter()
            .map(|row| row.event)
            .filter(|event| {
                event.channel_h == channel
                    && (event.created_at > after_created_at
                        || (event.created_at == after_created_at && event.id.as_str() > after_id))
            })
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            (&left.created_at, &left.id).cmp(&(&right.created_at, &right.id))
        });
        events.truncate(limit as usize);
        events
    }

    pub(crate) fn latest_chat_by_author(&self, channel: &str) -> HashMap<String, u64> {
        let mut latest = HashMap::new();
        for row in self.projected_events_for_kind_channel(KIND_CHAT, channel) {
            latest
                .entry(row.event.pubkey)
                .and_modify(|current: &mut u64| *current = (*current).max(row.event.created_at))
                .or_insert(row.event.created_at);
        }
        latest
    }

    pub(crate) fn count_chat_before(&self, channel: &str, before: u64) -> u32 {
        self.projected_events_for_kind_channel(KIND_CHAT, channel)
            .into_iter()
            .filter(|row| row.event.created_at < before)
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    pub(crate) fn events_by_kind(&self, kind: u16, limit: u32) -> Vec<RelayEvent> {
        let mut events = self
            .projected_events_for_kind(kind)
            .into_iter()
            .map(|row| row.event)
            .collect::<Vec<_>>();
        events.sort_by(|left, right| {
            (&right.created_at, &right.id).cmp(&(&left.created_at, &left.id))
        });
        events.truncate(limit as usize);
        events
    }
}
