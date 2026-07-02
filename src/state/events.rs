//! `relay_events` — verbatim cache of every relay event except the kinds that
//! have dedicated caches (0, 39xxx, 30315).
//!
//! NIP-01 replacement is applied ON INSERT:
//!   * addressable  (30000 <= kind < 40000): replace older by (kind, pubkey, d_tag)
//!   * replaceable  (kind == 0 || kind == 3 || 10000 <= kind < 20000): replace by (kind, pubkey)
//!   * regular: append.

use super::*;

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<RelayEvent> {
    Ok(RelayEvent {
        id: row.get(0)?,
        kind: row.get::<_, i64>(1)? as u32,
        pubkey: row.get(2)?,
        created_at: row.get(3)?,
        channel_h: row.get(4)?,
        d_tag: row.get(5)?,
        content: row.get(6)?,
        tags_json: row.get(7)?,
    })
}

const COLS: &str = "id, kind, pubkey, created_at, channel_h, d_tag, content, tags_json";

fn is_addressable(kind: u32) -> bool {
    (30000..40000).contains(&kind)
}

fn is_replaceable(kind: u32) -> bool {
    kind == 0 || kind == 3 || (10000..20000).contains(&kind)
}

impl Store {
    /// Insert a relay event applying NIP-01 replacement. Returns `true` if the
    /// event was stored (it was new and not superseded by a newer cached event),
    /// `false` if it was an older duplicate that lost the replacement race.
    pub fn insert_event(&self, ev: &RelayEvent) -> Result<bool> {
        // Replacement: an existing newer event for the same coordinate wins.
        if is_addressable(ev.kind) {
            let newer: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM relay_events
                     WHERE kind=?1 AND pubkey=?2 AND d_tag=?3 AND created_at >= ?4 LIMIT 1",
                    params![ev.kind as i64, ev.pubkey, ev.d_tag, ev.created_at],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if newer {
                return Ok(false);
            }
            self.conn.execute(
                "DELETE FROM relay_events WHERE kind=?1 AND pubkey=?2 AND d_tag=?3",
                params![ev.kind as i64, ev.pubkey, ev.d_tag],
            )?;
        } else if is_replaceable(ev.kind) {
            let newer: bool = self
                .conn
                .query_row(
                    "SELECT 1 FROM relay_events
                     WHERE kind=?1 AND pubkey=?2 AND created_at >= ?3 LIMIT 1",
                    params![ev.kind as i64, ev.pubkey, ev.created_at],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if newer {
                return Ok(false);
            }
            self.conn.execute(
                "DELETE FROM relay_events WHERE kind=?1 AND pubkey=?2",
                params![ev.kind as i64, ev.pubkey],
            )?;
        }
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO relay_events
                 (id, kind, pubkey, created_at, channel_h, d_tag, content, tags_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                ev.id,
                ev.kind as i64,
                ev.pubkey,
                ev.created_at,
                ev.channel_h,
                ev.d_tag,
                ev.content,
                ev.tags_json
            ],
        )?;
        Ok(n > 0)
    }

    /// Fetch one event by id.
    pub fn get_event(&self, id: &str) -> Result<Option<RelayEvent>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM relay_events WHERE id=?1"),
                params![id],
                row_to_event,
            )
            .optional()?)
    }

    /// Fetch one event by an unambiguous prefix of its id (agent-facing
    /// surfaces show only a short prefix to save tokens; see
    /// `crate::util::short_id`). `GLOB` (case-sensitive, index-friendly for a
    /// no-wildcard-prefix pattern) rather than `LIKE` since event ids are
    /// lowercase hex. Falls back to an exact match when `prefix` is already a
    /// full id. Bails loud on an ambiguous prefix rather than silently
    /// returning an arbitrary match.
    pub fn get_event_by_prefix(&self, prefix: &str) -> Result<Option<RelayEvent>> {
        if prefix.len() >= 64 {
            return self.get_event(prefix);
        }
        let pattern = format!("{prefix}*");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_events WHERE id GLOB ?1 LIMIT 2"
        ))?;
        let mut rows = stmt.query_map(params![pattern], row_to_event)?;
        let first = rows.next().transpose()?;
        if rows.next().is_some() {
            anyhow::bail!("ambiguous id prefix {prefix:?}: matches more than one message");
        }
        Ok(first)
    }

    /// True if an event id is already cached.
    pub fn has_event(&self, id: &str) -> Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM relay_events WHERE id=?1",
                params![id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// Chat log for a channel: events with `created_at > since`, oldest-first,
    /// capped at `limit`. Caller filters by kind if it only wants chat kinds.
    pub fn chat_for_channel(
        &self,
        channel_h: &str,
        since: u64,
        limit: u32,
    ) -> Result<Vec<RelayEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_events
             WHERE channel_h=?1 AND created_at > ?2
             ORDER BY created_at ASC, id ASC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![channel_h, since, limit], row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Chat log rows after an exact `(created_at, id)` cursor, oldest-first.
    /// This preserves same-second ordering for live catch-up without replaying
    /// rows at the cursor timestamp whose ids sort before or equal to the cursor.
    pub fn chat_for_channel_after(
        &self,
        channel_h: &str,
        after_created_at: u64,
        after_id: &str,
        limit: u32,
    ) -> Result<Vec<RelayEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_events
             WHERE channel_h=?1
               AND (created_at > ?2 OR (created_at = ?2 AND id > ?3))
             ORDER BY created_at ASC, id ASC LIMIT ?4"
        ))?;
        let rows = stmt.query_map(
            params![channel_h, after_created_at, after_id, limit],
            row_to_event,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Count kind:9 chat events in a channel with `created_at < before`. Used on
    /// first turn to tell a newly-joined session how much history it can't see.
    pub fn count_channel_events_before(&self, channel_h: &str, before: u64) -> Result<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM relay_events WHERE channel_h=?1 AND kind=9 AND created_at<?2",
            params![channel_h, before],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    /// Most recent events of a given kind, newest-first, capped at `limit`.
    pub fn events_by_kind(&self, kind: u32, limit: u32) -> Result<Vec<RelayEvent>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM relay_events WHERE kind=?1
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![kind as i64, limit], row_to_event)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

#[cfg(test)]
mod tests;
