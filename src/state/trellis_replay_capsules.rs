//! Durable Trellis replay capsules (§4.4): serialized input scripts captured at
//! the drive seam, bounded by row count and total script bytes.
//!
//! The ceilings are intentionally small enough for local-forensics retention, not
//! archival: newest 512 capsules and at most 16 MiB of serialized script JSON.

use super::*;

pub const REPLAY_CAPSULE_RETENTION_COUNT: i64 = 512;
pub const REPLAY_CAPSULE_RETENTION_BYTES: i64 = 16 * 1024 * 1024;

const COLS: &str = "id, surface, trigger_kind, trigger_ref, script_json, \
     script_bytes, format_version, created_at";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayCapsuleRow {
    pub id: i64,
    pub surface: String,
    pub trigger_kind: String,
    pub trigger_ref: String,
    pub script_json: String,
    pub script_bytes: i64,
    pub format_version: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReplayCapsule {
    pub surface: String,
    pub trigger_kind: String,
    pub trigger_ref: String,
    pub script_json: String,
    pub format_version: i64,
    pub created_at: i64,
}

fn row_to_capsule(row: &rusqlite::Row) -> rusqlite::Result<ReplayCapsuleRow> {
    Ok(ReplayCapsuleRow {
        id: row.get(0)?,
        surface: row.get(1)?,
        trigger_kind: row.get(2)?,
        trigger_ref: row.get(3)?,
        script_json: row.get(4)?,
        script_bytes: row.get(5)?,
        format_version: row.get(6)?,
        created_at: row.get(7)?,
    })
}

impl Store {
    pub fn record_replay_capsule(&self, row: &NewReplayCapsule) -> Result<i64> {
        let script_bytes = row.script_json.len() as i64;
        self.conn.execute(
            "INSERT INTO trellis_replay_capsules
                 (surface, trigger_kind, trigger_ref, script_json, script_bytes,
                  format_version, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.surface,
                row.trigger_kind,
                row.trigger_ref,
                row.script_json,
                script_bytes,
                row.format_version,
                row.created_at,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.prune_replay_capsules(
            REPLAY_CAPSULE_RETENTION_COUNT,
            REPLAY_CAPSULE_RETENTION_BYTES,
        )?;
        Ok(id)
    }

    pub fn get_replay_capsule(&self, id: i64) -> Result<Option<ReplayCapsuleRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM trellis_replay_capsules WHERE id=?1"),
                params![id],
                row_to_capsule,
            )
            .optional()?)
    }

    pub fn latest_replay_capsules(
        &self,
        surface: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ReplayCapsuleRow>> {
        let sql = match surface {
            Some(_) => format!(
                "SELECT {COLS} FROM trellis_replay_capsules
                 WHERE surface=?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
            ),
            None => format!(
                "SELECT {COLS} FROM trellis_replay_capsules
                 ORDER BY created_at DESC, id DESC LIMIT ?1"
            ),
        };
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = match surface {
            Some(surface) => stmt.query_map(params![surface, limit], row_to_capsule)?,
            None => stmt.query_map(params![limit], row_to_capsule)?,
        };
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn prune_replay_capsules(&self, max_count: i64, max_bytes: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM trellis_replay_capsules
             WHERE id NOT IN (
                 SELECT id FROM trellis_replay_capsules
                 ORDER BY created_at DESC, id DESC
                 LIMIT ?1
             )",
            params![max_count],
        )?;

        let mut total = self.replay_capsule_total_bytes()?;
        while total > max_bytes {
            let Some((id, bytes)) = self.oldest_replay_capsule()? else {
                break;
            };
            self.conn.execute(
                "DELETE FROM trellis_replay_capsules WHERE id=?1",
                params![id],
            )?;
            total = total.saturating_sub(bytes);
        }
        Ok(())
    }

    fn replay_capsule_total_bytes(&self) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(script_bytes), 0) FROM trellis_replay_capsules",
            [],
            |row| row.get(0),
        )?)
    }

    fn oldest_replay_capsule(&self) -> Result<Option<(i64, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, script_bytes FROM trellis_replay_capsules
                 ORDER BY created_at ASC, id ASC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(n: i64, bytes: usize) -> NewReplayCapsule {
        NewReplayCapsule {
            surface: "status".into(),
            trigger_kind: "tick".into(),
            trigger_ref: format!("s{n}"),
            script_json: "x".repeat(bytes),
            format_version: 1,
            created_at: n,
        }
    }

    #[test]
    fn records_and_fetches_capsule() {
        let s = Store::open_memory().unwrap();
        let id = s.record_replay_capsule(&row(1, 3)).unwrap();
        let got = s.get_replay_capsule(id).unwrap().unwrap();
        assert_eq!(got.surface, "status");
        assert_eq!(got.script_bytes, 3);
    }

    #[test]
    fn retention_prunes_by_count_and_bytes() {
        let s = Store::open_memory().unwrap();
        for n in 0..5 {
            s.conn
                .execute(
                    "INSERT INTO trellis_replay_capsules
                     (surface, trigger_kind, trigger_ref, script_json, script_bytes,
                      format_version, created_at)
                     VALUES ('status', 'tick', '', ?1, ?2, 1, ?3)",
                    params!["x".repeat(10), 10_i64, n],
                )
                .unwrap();
        }
        s.prune_replay_capsules(3, 25).unwrap();
        let rows = s.latest_replay_capsules(None, 10).unwrap();
        assert_eq!(rows.len(), 2, "byte ceiling prunes beyond count ceiling");
        assert_eq!(
            rows.iter().map(|r| r.created_at).collect::<Vec<_>>(),
            vec![4, 3]
        );
    }
}
