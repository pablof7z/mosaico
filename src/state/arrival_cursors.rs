//! Durable host-local arrival cursors for session admission fences.
//!
//! This is not a relay-state cache: it stores only an event id and the order in
//! which this daemon first observed it. NMP remains the sole owner of the event.

use super::*;

impl Store {
    pub(crate) fn record_nmp_arrival(&self, event_id: &str) -> Result<u64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO nmp_event_arrivals (event_id) VALUES (?1)",
            [event_id],
        )?;
        self.nmp_arrival_sequence(event_id)?
            .context("NMP arrival row disappeared after insertion")
    }

    pub(crate) fn nmp_arrival_sequence(&self, event_id: &str) -> Result<Option<u64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT sequence FROM nmp_event_arrivals WHERE event_id=?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub(crate) fn latest_nmp_arrival_sequence(&self) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM nmp_event_arrivals",
            [],
            |row| row.get(0),
        )?)
    }
}
