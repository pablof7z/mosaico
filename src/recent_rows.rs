//! Bounded sync-readable recent-rows store — the replacement for the
//! `FavaViews` row mirror's `OwnedRow`/index layer.
//!
//! The daemon retains the recent activity per channel (chat, reactions,
//! status — every `#h`-bearing row the live observations deliver) and exposes
//! it synchronously to the `state/` read layer. Rows beyond the per-channel cap
//! are evicted oldest-first; a miss here means "older than the retained
//! window," and a caller that needs it fetches from Fava on demand. This is not
//! the O(history) mirror: there is no boot replay of the whole store — only
//! the bounded recent set the live drain maintains.
//!
//! Observations overlap: one kind:9 chat in channel C that mentions pubkey P is
//! owned by both `mosaico-h-C` and `mosaico-p-P`. A row only leaves the retained
//! union once *every* observation that owned it has detached it, so the writer
//! is observation-scoped (`add`/`remove` carry an `observation_id`) and the
//! store refcounts owners per id — the same union semantics the old mirror had,
//! in bounded form. The router (slice 5) decodes `RowDelta` per observation and
//! calls these writers; generation/stale-frame guards live in the router, not
//! here.
//!
//! Read: every method is sync and lock-bounded.

use std::collections::{BTreeMap, BTreeSet};

use crate::state::RelayEvent;

/// Recent rows retained per channel. Covers the hot read paths (channel tail,
/// ambient chat, reply-nudge, capture activity) without holding unbounded
/// history. Older rows are an on-demand Fava fetch, not a mirror.
const CAP_PER_CHANNEL: usize = 10_000;

pub(crate) struct RecentRows {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// Canonical union: `id -> (event, owning observation ids)`. A row is
    /// retained while at least one observation still owns it.
    rows: BTreeMap<String, (RelayEvent, BTreeSet<String>)>,
    /// `channel_h -> (created_at, id)` ascending, for per-channel listings.
    by_channel: BTreeMap<String, BTreeSet<(u64, String)>>,
    /// `kind -> (created_at, id)` ascending; read reversed for "recent first".
    by_kind: BTreeMap<u32, BTreeSet<(u64, String)>>,
    /// `observation_id -> ids it owns`, so `close`/`replace` can detach a
    /// whole observation's contribution without scanning the union.
    by_observation: BTreeMap<String, BTreeSet<String>>,
}

impl Default for RecentRows {
    fn default() -> Self {
        Self {
            inner: std::sync::Mutex::new(Inner::default()),
        }
    }
}

impl RecentRows {
    /// One observation gained a row. Refcounts owners; indexes the row only on
    /// the 0->1 union entry. `RowDelta::Added` and `RowDelta::Updated` both
    /// route here (an `Updated` only grows provenance, so the row is already
    /// present — this is a no-op refcount bump).
    pub(crate) fn add(&self, observation_id: &str, event: RelayEvent) {
        let mut inner = self.inner.lock().expect("recent rows poisoned");
        let id = event.id.clone();
        let kind = event.kind;
        let created_at = event.created_at;
        let channel_h = event.channel_h.clone();

        let entered = inner
            .rows
            .entry(id.clone())
            .or_insert_with(|| (event.clone(), BTreeSet::new()))
            .1
            .insert(observation_id.to_string());
        inner
            .by_observation
            .entry(observation_id.to_string())
            .or_default()
            .insert(id.clone());
        if entered {
            inner
                .by_kind
                .entry(kind)
                .or_default()
                .insert((created_at, id.clone()));
            if !channel_h.is_empty() {
                let to_evict = {
                    let set = inner.by_channel.entry(channel_h).or_default();
                    set.insert((created_at, id));
                    let mut evicted = None;
                    while set.len() > CAP_PER_CHANNEL {
                        let oldest = set.iter().next().expect("nonempty").clone();
                        set.remove(&oldest);
                        evicted = Some(oldest.1);
                    }
                    evicted
                };
                if let Some(evicted_id) = to_evict {
                    Self::evict(&mut inner, evicted_id);
                }
            }
        }
    }

    /// One observation lost a row (`RowDelta::Removed`). Drops the row from the
    /// union only when the last owner detaches.
    pub(crate) fn remove(&self, observation_id: &str, id: &str) {
        let mut inner = self.inner.lock().expect("recent rows poisoned");
        Self::detach_owner(&mut inner, observation_id, id);
    }

    /// Drop every row an observation owns — used on `close`/`replace` when a
    /// whole observation's contribution leaves the union at once.
    pub(crate) fn close_observation(&self, observation_id: &str) {
        let mut inner = self.inner.lock().expect("recent rows poisoned");
        let ids = inner
            .by_observation
            .remove(observation_id)
            .unwrap_or_default();
        for id in ids {
            Self::detach_owner(&mut inner, observation_id, &id);
        }
    }

    /// Forget everything (shutdown/teardown; not a per-frame op).
    pub(crate) fn clear(&self) {
        let mut inner = self.inner.lock().expect("recent rows poisoned");
        inner.rows.clear();
        inner.by_channel.clear();
        inner.by_kind.clear();
        inner.by_observation.clear();
    }

    /// Point lookup against the current union.
    pub(crate) fn event(&self, id: &str) -> Option<RelayEvent> {
        self.inner
            .lock()
            .expect("recent rows poisoned")
            .rows
            .get(id)
            .map(|(event, _)| event.clone())
    }

    /// One event whose id starts with `prefix`, or `None`. Ambiguous prefixes
    /// (more than one match) are an error, matching the old mirror's contract.
    pub(crate) fn event_by_prefix(&self, prefix: &str) -> anyhow::Result<Option<RelayEvent>> {
        let inner = self.inner.lock().expect("recent rows poisoned");
        let mut matches = inner
            .rows
            .range(prefix.to_string()..)
            .take_while(|(id, _)| id.starts_with(prefix));
        let first = matches.next().map(|(_, (event, _))| event.clone());
        if matches.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one event");
        }
        Ok(first)
    }

    /// Per-channel rows in ascending `(created_at, id)` order, `since`-filtered
    /// and capped at `limit`.
    pub(crate) fn events_in_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Vec<RelayEvent> {
        let inner = self.inner.lock().expect("recent rows poisoned");
        let Some(set) = inner.by_channel.get(channel_h) else {
            return Vec::new();
        };
        set.iter()
            .skip_while(|(created_at, _)| *created_at < since)
            .take(limit as usize)
            .filter_map(|(_, id)| inner.rows.get(id).map(|(event, _)| event.clone()))
            .collect()
    }

    /// Recent-first rows of one kind, capped at `limit`.
    pub(crate) fn events_by_kind(&self, kind: u16, limit: u32) -> Vec<RelayEvent> {
        let inner = self.inner.lock().expect("recent rows poisoned");
        let Some(set) = inner.by_kind.get(&(kind as u32)) else {
            return Vec::new();
        };
        set.iter()
            .rev()
            .take(limit as usize)
            .filter_map(|(_, id)| inner.rows.get(id).map(|(event, _)| event.clone()))
            .collect()
    }

    /// All retained rows of one kind in one channel, ascending — the substrate
    /// for `messages()`, `reactions()`, and status projections (which decode
    /// `RelayEvent` into their own shapes in the state layer).
    pub(crate) fn events_for_kind_channel(&self, kind: u16, channel_h: &str) -> Vec<RelayEvent> {
        let inner = self.inner.lock().expect("recent rows poisoned");
        let kind = kind as u32;
        inner
            .by_channel
            .get(channel_h)
            .into_iter()
            .flatten()
            .filter_map(|(_, id)| inner.rows.get(id).map(|(event, _)| event.clone()))
            .filter(|event| event.kind == kind)
            .collect()
    }

    /// Last `created_at` per author in one channel, for activity presence.
    pub(crate) fn latest_chat_by_author(
        &self,
        channel_h: &str,
        chat_kind: u16,
    ) -> BTreeMap<String, u64> {
        let mut latest: BTreeMap<String, u64> = BTreeMap::new();
        for event in self.events_for_kind_channel(chat_kind, channel_h) {
            latest
                .entry(event.pubkey)
                .and_modify(|current| *current = (*current).max(event.created_at))
                .or_insert(event.created_at);
        }
        latest
    }

    /// Remove `id`'s owner `observation_id`; if no owners remain, drop the row
    /// from the union and all indices.
    fn detach_owner(inner: &mut Inner, observation_id: &str, id: &str) {
        let Some((_event, owners)) = inner.rows.get_mut(id) else {
            return;
        };
        owners.remove(observation_id);
        if !owners.is_empty() {
            return;
        }
        let (event, _) = inner
            .rows
            .remove(id)
            .expect("row present with empty owners");
        remove_index_entry(
            &mut inner.by_kind,
            event.kind,
            (event.created_at, event.id.clone()),
        );
        if !event.channel_h.is_empty() {
            remove_index_entry(
                &mut inner.by_channel,
                event.channel_h,
                (event.created_at, event.id),
            );
        }
    }

    /// Evict a row by id from the union and every index (channel cap overflow).
    fn evict(inner: &mut Inner, id: String) {
        let Some((event, owners)) = inner.rows.remove(&id) else {
            return;
        };
        remove_index_entry(
            &mut inner.by_kind,
            event.kind,
            (event.created_at, event.id.clone()),
        );
        if !event.channel_h.is_empty() {
            remove_index_entry(
                &mut inner.by_channel,
                event.channel_h,
                (event.created_at, event.id.clone()),
            );
        }
        for observation_id in &owners {
            if let Some(set) = inner.by_observation.get_mut(observation_id) {
                set.remove(&id);
            }
        }
    }
}

fn remove_index_entry<K: Ord + Clone>(
    index: &mut BTreeMap<K, BTreeSet<(u64, String)>>,
    key: K,
    entry: (u64, String),
) {
    if let Some(set) = index.get_mut(&key) {
        set.remove(&entry);
        if set.is_empty() {
            index.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relay(id: &str, kind: u32, channel: &str, created_at: u64) -> RelayEvent {
        RelayEvent {
            id: id.into(),
            kind,
            pubkey: format!("pk-{id}"),
            created_at,
            channel_h: channel.into(),
            d_tag: String::new(),
            content: id.into(),
            tags_json: "[]".into(),
        }
    }

    #[test]
    fn add_then_event_returns_the_row() {
        let rows = RecentRows::default();
        rows.add("obs-a", relay("a", 9, "room", 10));
        assert_eq!(rows.event("a").unwrap().created_at, 10);
    }

    #[test]
    fn remove_from_one_of_two_owners_keeps_the_row() {
        let rows = RecentRows::default();
        rows.add("mosaico-h-room", relay("a", 9, "room", 10));
        rows.add("mosaico-p-pk-a", relay("a", 9, "room", 10)); // same row, second owner
                                                               // Detaching the mention observation must NOT drop the row: the channel
                                                               // observation still owns it.
        rows.remove("mosaico-p-pk-a", "a");
        assert!(rows.event("a").is_some());
        assert_eq!(rows.events_in_channel("room", 0, 10).len(), 1);
        // Now detach the last owner: the row leaves the union.
        rows.remove("mosaico-h-room", "a");
        assert!(rows.event("a").is_none());
        assert!(rows.events_in_channel("room", 0, 10).is_empty());
    }

    #[test]
    fn close_observation_drops_only_its_rows() {
        let rows = RecentRows::default();
        rows.add("obs-a", relay("a", 9, "room", 10));
        rows.add("obs-b", relay("b", 9, "room", 20));
        rows.close_observation("obs-a");
        assert!(rows.event("a").is_none());
        assert!(rows.event("b").is_some());
    }

    #[test]
    fn close_keeps_a_row_still_owned_elsewhere() {
        let rows = RecentRows::default();
        rows.add("obs-a", relay("a", 9, "room", 10));
        rows.add("obs-b", relay("a", 9, "room", 10));
        rows.close_observation("obs-a");
        assert!(
            rows.event("a").is_some(),
            "row survives while obs-b owns it"
        );
        rows.close_observation("obs-b");
        assert!(rows.event("a").is_none());
    }

    #[test]
    fn events_in_channel_orders_ascending_filters_since_and_caps() {
        let rows = RecentRows::default();
        rows.add("o", relay("a", 9, "room", 5));
        rows.add("o", relay("b", 9, "room", 10));
        rows.add("o", relay("c", 9, "room", 10)); // same second: id breaks the tie
        rows.add("o", relay("d", 9, "other", 20));
        let ids: Vec<_> = rows
            .events_in_channel("room", 10, 10)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["b", "c"]); // since=10 excludes a; d is in "other"
        assert_eq!(rows.events_in_channel("room", 0, 1).len(), 1);
    }

    #[test]
    fn events_by_kind_is_recent_first_and_capped() {
        let rows = RecentRows::default();
        rows.add("o", relay("a", 9, "room", 5));
        rows.add("o", relay("b", 9, "room", 10));
        rows.add("o", relay("c", 7, "room", 99));
        let chat: Vec<_> = rows
            .events_by_kind(9, 10)
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(chat, vec!["b", "a"]); // desc by created_at
        assert!(rows.events_by_kind(7, 10).iter().any(|e| e.id == "c"));
        assert_eq!(rows.events_by_kind(9, 1).len(), 1);
    }

    #[test]
    fn event_by_prefix_matches_unique_and_rejects_ambiguous() {
        let rows = RecentRows::default();
        rows.add("o", relay("abc123", 9, "room", 1));
        rows.add("o", relay("def456", 9, "room", 1));
        assert_eq!(rows.event_by_prefix("abc").unwrap().unwrap().id, "abc123");
        assert!(rows.event_by_prefix("xyz").unwrap().is_none());
        rows.add("o", relay("abc999", 9, "room", 2));
        assert!(rows.event_by_prefix("abc").is_err());
    }

    #[test]
    fn channel_cap_evicts_oldest_even_with_owners() {
        let rows = RecentRows::default();
        for i in 0..=CAP_PER_CHANNEL {
            rows.add("o", relay(&format!("e{i}"), 9, "cap", i as u64));
        }
        assert!(rows.event("e0").is_none(), "oldest evicted");
        assert!(rows.event(&format!("e{CAP_PER_CHANNEL}")).is_some());
        assert_eq!(
            rows.events_in_channel("cap", 0, u32::MAX).len(),
            CAP_PER_CHANNEL
        );
    }
}
