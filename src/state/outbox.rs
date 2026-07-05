//! `outbox` — the outbound publish queue.
//!
//! A signed event is enqueued before it hits the relay so it survives a crash
//! between the decision to publish and the relay ack. The drainer publishes
//! pending rows and marks each published or failed (bumping the retry count).

use super::*;

const COLS: &str = "local_id, event_json, state, retries, last_error, enqueued_at";

fn row_to_outbox(row: &rusqlite::Row) -> rusqlite::Result<OutboxRow> {
    Ok(OutboxRow {
        local_id: row.get(0)?,
        event_json: row.get(1)?,
        state: row.get(2)?,
        retries: row.get(3)?,
        last_error: row.get(4)?,
        enqueued_at: row.get(5)?,
    })
}

impl Store {
    /// Queue a signed event JSON for publishing. Returns its `local_id`.
    pub fn enqueue_outbox(&self, event_json: &str, enqueued_at: u64) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO outbox (event_json, state, enqueued_at) VALUES (?1, 'pending', ?2)",
            params![event_json, enqueued_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Read-only peek at the next pending rows to publish, oldest-first, capped
    /// at `limit`. Callers mark rows published or failed after the relay result.
    pub fn peek_outbox(&self, limit: u32) -> Result<Vec<OutboxRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM outbox WHERE state='pending' ORDER BY local_id ASC LIMIT ?1"
        ))?;
        let rows = stmt.query_map(params![limit], row_to_outbox)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Apply a Trellis-derived publish result to the durable queue row.
    pub fn apply_outbox_projection(
        &self,
        local_id: i64,
        state: &str,
        last_error: Option<&str>,
        bump_retries: bool,
    ) -> Result<()> {
        if bump_retries {
            self.conn.execute(
                "UPDATE outbox
                 SET state=?2, retries=retries+1, last_error=?3
                 WHERE local_id=?1",
                params![local_id, state, last_error],
            )?;
        } else {
            self.conn.execute(
                "UPDATE outbox SET state=?2, last_error=?3 WHERE local_id=?1",
                params![local_id, state, last_error],
            )?;
        }
        Ok(())
    }
}
