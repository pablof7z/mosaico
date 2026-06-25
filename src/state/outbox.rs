use super::{StatusOutboxDebugRow, Store};
use anyhow::Result;
use rusqlite::params;

impl Store {
    pub fn list_status_outbox_debug(&self, limit: u64) -> Result<Vec<StatusOutboxDebugRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT
               o.session_id,
               o.state_version,
               o.publish_state,
               o.retries,
               o.native_event_id,
               o.last_error,
               o.enqueued_at,
               COALESCE(s.agent_slug, ''),
               COALESCE(s.project, ''),
               COALESCE(s.title, ''),
               COALESCE(s.activity, ''),
               COALESCE(s.busy, 0)
             FROM status_outbox o
             LEFT JOIN session_state s ON s.session_id=o.session_id
             ORDER BY o.enqueued_at DESC, o.state_version DESC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit.min(i64::MAX as u64) as i64], |r| {
            Ok(StatusOutboxDebugRow {
                session_id: r.get(0)?,
                state_version: r.get(1)?,
                publish_state: r.get(2)?,
                retries: r.get(3)?,
                native_event_id: r.get(4)?,
                last_error: r.get(5)?,
                enqueued_at: r.get(6)?,
                agent_slug: r.get(7)?,
                project: r.get(8)?,
                title: r.get(9)?,
                activity: r.get(10)?,
                busy: r.get::<_, i64>(11)? != 0,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}
