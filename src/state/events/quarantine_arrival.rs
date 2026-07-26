use super::*;

impl Store {
    /// Reserve this event's true local arrival position without exposing its
    /// body before admission. Replaying an admitted quarantine later activates
    /// this same row, preserving the membership sequence fence.
    pub(crate) fn reserve_quarantined_event_arrival(&self, ev: &RelayEvent) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO relay_events
                 (id, kind, pubkey, created_at, channel_h, d_tag, content, tags_json)
             VALUES (?1, ?2, ?3, ?4, ?5, '', '', '[]')",
            params![
                ev.id,
                QUARANTINED_EVENT_KIND as i64,
                ev.pubkey,
                ev.created_at,
                ev.channel_h
            ],
        )?;
        Ok(n > 0)
    }

    pub(crate) fn activate_quarantined_event(&self, ev: &RelayEvent) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE relay_events
             SET kind=?2, pubkey=?3, created_at=?4, channel_h=?5,
                 d_tag=?6, content=?7, tags_json=?8
             WHERE id=?1 AND kind=?9",
            params![
                ev.id,
                ev.kind as i64,
                ev.pubkey,
                ev.created_at,
                ev.channel_h,
                ev.d_tag,
                ev.content,
                ev.tags_json,
                QUARANTINED_EVENT_KIND as i64,
            ],
        )?;
        if n > 0 {
            Ok(true)
        } else {
            self.insert_event(ev)
        }
    }

    pub(crate) fn remove_quarantined_event_arrival(&self, id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM relay_events WHERE id=?1 AND kind=?2",
            params![id, QUARANTINED_EVENT_KIND as i64],
        )?;
        Ok(n > 0)
    }
}
