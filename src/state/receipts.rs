//! `receipts` — persisted, Trellis-vocabulary-free reconciler receipts (Slice 8:
//! retrospective instrumentation).
//!
//! Each Trellis reconciler commit yields a `TransactionResult`; the caller
//! flattens it into plain fields (no `ResourceKey`/`ResourcePlan`/`Graph`
//! types cross this boundary) before calling [`Store::record_receipt`]. This
//! lets "why did this hook context/status/subscription set have this shape"
//! be answered later without depending on Trellis's in-memory types.
//!
//! `transaction_id` is stored as `INTEGER`: Trellis's `TransactionId` newtypes
//! a `u64` monotonic counter (see `trellis-core::ids::TransactionId`), which
//! fits an `i64` column for the lifetime of any realistic run. Callers cast
//! `TransactionId`/`Revision` to `i64` when flattening. Callers pass
//! `created_at`; this module never reads the clock.

use super::*;

const COLS: &str = "id, surface, transaction_id, revision, changed_summary, \
     commands, artifact_ref, created_at";

/// One persisted reconciler receipt, flattened to plain fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptRow {
    pub id: i64,
    /// Which surface this receipt explains: `"subscriptions"` | `"status"` |
    /// `"hook_context"`.
    pub surface: String,
    pub transaction_id: i64,
    pub revision: i64,
    /// JSON string built by the caller describing what changed.
    pub changed_summary: String,
    /// JSON array of plain command records: `{kind, key, reason}`.
    pub commands: String,
    /// e.g. the published event id or hook-call id this receipt explains.
    pub artifact_ref: Option<String>,
    pub created_at: i64,
}

/// Input shape for recording a new receipt. `id` is assigned by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewReceipt {
    pub surface: String,
    pub transaction_id: i64,
    pub revision: i64,
    pub changed_summary: String,
    pub commands: String,
    pub artifact_ref: Option<String>,
    pub created_at: i64,
}

fn row_to_receipt(row: &rusqlite::Row) -> rusqlite::Result<ReceiptRow> {
    Ok(ReceiptRow {
        id: row.get(0)?,
        surface: row.get(1)?,
        transaction_id: row.get(2)?,
        revision: row.get(3)?,
        changed_summary: row.get(4)?,
        commands: row.get(5)?,
        artifact_ref: row.get(6)?,
        created_at: row.get(7)?,
    })
}

impl Store {
    /// Record one flattened reconciler receipt. Returns the assigned `id`.
    pub fn record_receipt(&self, row: &NewReceipt) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO receipts
                 (surface, transaction_id, revision, changed_summary, commands,
                  artifact_ref, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.surface,
                row.transaction_id,
                row.revision,
                row.changed_summary,
                row.commands,
                row.artifact_ref,
                row.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch one receipt by id.
    pub fn get_receipt(&self, id: i64) -> Result<Option<ReceiptRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM receipts WHERE id=?1"),
                params![id],
                row_to_receipt,
            )
            .optional()?)
    }

    /// All receipts explaining a given artifact (published event id / hook-call
    /// id), oldest first.
    pub fn receipts_by_artifact_ref(&self, artifact_ref: &str) -> Result<Vec<ReceiptRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM receipts
             WHERE artifact_ref=?1
             ORDER BY created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![artifact_ref], row_to_receipt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Most recent receipts for a surface, newest first, capped at `limit`.
    pub fn latest_receipts_for_surface(
        &self,
        surface: &str,
        limit: u32,
    ) -> Result<Vec<ReceiptRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM receipts
             WHERE surface=?1
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![surface, limit], row_to_receipt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The receipt for `surface` whose `created_at` is closest to `at_millis`.
    pub fn find_receipt_near(&self, surface: &str, at_millis: i64) -> Result<Option<ReceiptRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM receipts
                     WHERE surface=?1
                     ORDER BY ABS(created_at - ?2) ASC, created_at ASC LIMIT 1"
                ),
                params![surface, at_millis],
                row_to_receipt,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{receipts::NewReceipt, Store};

    fn receipt(surface: &str, artifact_ref: Option<&str>, created_at: i64) -> NewReceipt {
        NewReceipt {
            surface: surface.into(),
            transaction_id: 42,
            revision: 7,
            changed_summary: r#"{"added":1,"removed":0}"#.into(),
            commands: r#"[{"kind":"publish","key":"k1","reason":"changed"}]"#.into(),
            artifact_ref: artifact_ref.map(str::to_string),
            created_at,
        }
    }

    #[test]
    fn record_then_get_round_trips() {
        let s = Store::open_memory().unwrap();
        let id = s
            .record_receipt(&receipt("status", Some("evt-1"), 1_000))
            .unwrap();

        let row = s.get_receipt(id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.surface, "status");
        assert_eq!(row.transaction_id, 42);
        assert_eq!(row.revision, 7);
        assert_eq!(row.artifact_ref.as_deref(), Some("evt-1"));
        assert_eq!(row.created_at, 1_000);
    }

    #[test]
    fn get_missing_id_returns_none() {
        let s = Store::open_memory().unwrap();
        assert!(s.get_receipt(999).unwrap().is_none());
    }

    #[test]
    fn latest_for_surface_orders_newest_first_and_respects_limit() {
        let s = Store::open_memory().unwrap();
        s.record_receipt(&receipt("status", Some("evt-1"), 1_000))
            .unwrap();
        s.record_receipt(&receipt("status", Some("evt-2"), 3_000))
            .unwrap();
        s.record_receipt(&receipt("status", Some("evt-3"), 2_000))
            .unwrap();
        // Different surface must not leak in.
        s.record_receipt(&receipt("hook_context", Some("evt-4"), 4_000))
            .unwrap();

        let rows = s.latest_receipts_for_surface("status", 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].created_at, 3_000);
        assert_eq!(rows[1].created_at, 2_000);
    }

    #[test]
    fn by_artifact_ref_filters_and_orders_oldest_first() {
        let s = Store::open_memory().unwrap();
        s.record_receipt(&receipt("status", Some("evt-a"), 2_000))
            .unwrap();
        s.record_receipt(&receipt("hook_context", Some("evt-a"), 1_000))
            .unwrap();
        s.record_receipt(&receipt("subscriptions", Some("evt-b"), 500))
            .unwrap();

        let rows = s.receipts_by_artifact_ref("evt-a").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].created_at, 1_000);
        assert_eq!(rows[1].created_at, 2_000);
    }

    #[test]
    fn find_near_picks_closest_created_at() {
        let s = Store::open_memory().unwrap();
        s.record_receipt(&receipt("hook_context", Some("evt-1"), 1_000))
            .unwrap();
        s.record_receipt(&receipt("hook_context", Some("evt-2"), 5_000))
            .unwrap();
        s.record_receipt(&receipt("hook_context", Some("evt-3"), 9_000))
            .unwrap();

        let row = s.find_receipt_near("hook_context", 6_000).unwrap().unwrap();
        assert_eq!(row.artifact_ref.as_deref(), Some("evt-2"));

        // No rows for an unknown surface.
        assert!(s
            .find_receipt_near("subscriptions", 6_000)
            .unwrap()
            .is_none());
    }
}
