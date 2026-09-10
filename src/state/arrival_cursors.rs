//! Durable host-local arrival cursors for session admission fences.
//!
//! This is not a relay-state cache: it stores only an event id and the order in
//! which this daemon first observed it. Nostr runtime remains the sole owner of the event.

use super::*;

impl Store {
    pub(crate) fn claim_event_arrival(&self, event_id: &str) -> Result<(u64, bool)> {
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO event_arrivals (event_id) VALUES (?1)",
            [event_id],
        )? > 0;
        let sequence = self
            .event_arrival_sequence(event_id)?
            .context("event arrival row disappeared after insertion")?;
        Ok((sequence, inserted))
    }

    #[cfg(test)]
    pub(crate) fn record_event_arrival(&self, event_id: &str) -> Result<u64> {
        self.claim_event_arrival(event_id)
            .map(|(sequence, _)| sequence)
    }

    pub(crate) fn event_arrival_sequence(&self, event_id: &str) -> Result<Option<u64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT sequence FROM event_arrivals WHERE event_id=?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn latest_event_arrival_sequence(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM event_arrivals",
            [],
            |row| row.get(0),
        )?)
    }
}
