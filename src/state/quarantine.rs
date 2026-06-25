use super::Store;
use anyhow::Result;
use rusqlite::params;

/// One row from the `inbound_quarantine` table, returned by `replay_quarantine`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantinedEnvelope {
    pub native_event_id: String,
    pub project_id: Option<String>,
    pub reason: String,
    pub raw_envelope: String,
    pub created_at: u64,
}

impl Store {
    /// Park an inbound event that could not be admitted yet. Idempotent on
    /// `native_event_id`.
    pub fn quarantine_inbound(
        &self,
        native_event_id: &str,
        project_id: Option<&str>,
        reason: &str,
        raw_envelope: &str,
        ts: u64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO inbound_quarantine
               (native_event_id, project_id, reason, raw_envelope, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![native_event_id, project_id, reason, raw_envelope, ts],
        )?;
        Ok(())
    }

    /// Quarantined envelopes awaiting replay, optionally filtered to one project.
    pub fn replay_quarantine(&self, project_id: Option<&str>) -> Result<Vec<QuarantinedEnvelope>> {
        let mut stmt = self.conn.prepare(
            "SELECT native_event_id, project_id, reason, raw_envelope, created_at
             FROM inbound_quarantine
             WHERE (?1 IS NULL OR project_id=?1)
             ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map(params![project_id], |r| {
                Ok(QuarantinedEnvelope {
                    native_event_id: r.get(0)?,
                    project_id: r.get(1)?,
                    reason: r.get(2)?,
                    raw_envelope: r.get(3)?,
                    created_at: r.get(4)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Drop a quarantined envelope once it has been replayed/admitted.
    pub fn clear_quarantine(&self, native_event_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM inbound_quarantine WHERE native_event_id=?1",
            params![native_event_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase1_quarantine_roundtrip_and_idempotent() {
        let s = Store::open_memory().unwrap();
        s.quarantine_inbound("evt-q", Some("proj-x"), "unhydrated", "{\"raw\":1}", 5)
            .unwrap();
        s.quarantine_inbound("evt-q", Some("proj-x"), "unhydrated", "{\"raw\":1}", 9)
            .unwrap();
        let all = s.replay_quarantine(None).unwrap();
        assert_eq!(all.len(), 1, "INSERT OR IGNORE dedups by native_event_id");
        assert_eq!(all[0].project_id.as_deref(), Some("proj-x"));
        assert!(s.replay_quarantine(Some("nope")).unwrap().is_empty());
        assert_eq!(s.replay_quarantine(Some("proj-x")).unwrap().len(), 1);
        s.clear_quarantine("evt-q").unwrap();
        assert!(s.replay_quarantine(None).unwrap().is_empty());
    }
}
