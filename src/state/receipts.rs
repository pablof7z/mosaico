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

    /// Receipts for an exact artifact id or unambiguous artifact id prefix.
    /// Multiple receipts for the same artifact are expected; multiple distinct
    /// artifacts sharing the prefix are ambiguous and must be named more fully.
    pub fn receipts_by_artifact_ref_prefix(&self, prefix: &str) -> Result<Vec<ReceiptRow>> {
        if prefix.len() >= 64 {
            return self.receipts_by_artifact_ref(prefix);
        }
        let pattern = format!("{prefix}*");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM receipts
             WHERE artifact_ref GLOB ?1
             ORDER BY created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![pattern], row_to_receipt)?;
        let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut artifact_refs = rows
            .iter()
            .filter_map(|r| r.artifact_ref.as_deref())
            .collect::<Vec<_>>();
        artifact_refs.sort_unstable();
        artifact_refs.dedup();
        if artifact_refs.len() > 1 {
            anyhow::bail!("ambiguous artifact prefix {prefix:?}: matches more than one event");
        }
        Ok(rows)
    }

    /// Most recent hook-context receipts for one pubkey, newest first.
    pub fn latest_hook_receipts_for_pubkey(
        &self,
        pubkey: &str,
        limit: u32,
    ) -> Result<Vec<ReceiptRow>> {
        let pattern = format!("{}:%", escape_like(pubkey));
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM receipts
             WHERE surface='hook_context' AND artifact_ref LIKE ?1 ESCAPE '\\'
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![pattern, limit], row_to_receipt)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Hook-context receipt for one pubkey nearest to `at_millis`.
    pub fn find_hook_receipt_for_pubkey_near(
        &self,
        pubkey: &str,
        at_millis: i64,
    ) -> Result<Option<ReceiptRow>> {
        let pattern = format!("{}:%", escape_like(pubkey));
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM receipts
                     WHERE surface='hook_context' AND artifact_ref LIKE ?1 ESCAPE '\\'
                     ORDER BY ABS(created_at - ?2) ASC, created_at DESC, id DESC LIMIT 1"
                ),
                params![pattern, at_millis],
                row_to_receipt,
            )
            .optional()?)
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

    /// Receipts for one surface transaction id, newest first. Transaction ids
    /// can repeat across daemon epochs, so callers must decide whether multiple
    /// rows are acceptable evidence or an ambiguity.
    pub fn receipts_for_surface_transaction(
        &self,
        surface: &str,
        transaction_id: i64,
    ) -> Result<Vec<ReceiptRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM receipts
             WHERE surface=?1 AND transaction_id=?2
             ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map(params![surface, transaction_id], row_to_receipt)?;
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

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

#[cfg(test)]
mod tests;
