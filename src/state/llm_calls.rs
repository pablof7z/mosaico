//! `llm_calls` — ledger of every distill LLM round-trip (Slice 8: retrospective
//! instrumentation).
//!
//! `distill::distill_session` makes one model call per invocation: a system
//! prompt, a transcript slice, and (on success) a raw response parsed into
//! `TITLE:`/`NOW:` lines. This store persists every round-trip verbatim so a
//! later "why did status say X" question can be answered exactly: what went
//! in, what came back, how it was parsed. Callers pass `created_at`; this
//! module never reads the clock.

use super::*;

const COLS: &str = "id, pubkey, window_hash, provider, model, system_prompt, \
     transcript_slice, raw_response, parsed_title, parsed_activity, created_at";

/// One persisted distill LLM round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmCallRow {
    pub id: i64,
    pub pubkey: String,
    pub window_hash: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub transcript_slice: String,
    pub raw_response: String,
    pub parsed_title: Option<String>,
    pub parsed_activity: Option<String>,
    pub created_at: i64,
}

/// Input shape for recording a new round-trip. `id` is assigned by the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLlmCall {
    pub pubkey: String,
    pub window_hash: String,
    pub provider: String,
    pub model: String,
    pub system_prompt: String,
    pub transcript_slice: String,
    pub raw_response: String,
    pub parsed_title: Option<String>,
    pub parsed_activity: Option<String>,
    pub created_at: i64,
}

fn row_to_llm_call(row: &rusqlite::Row) -> rusqlite::Result<LlmCallRow> {
    Ok(LlmCallRow {
        id: row.get(0)?,
        pubkey: row.get(1)?,
        window_hash: row.get(2)?,
        provider: row.get(3)?,
        model: row.get(4)?,
        system_prompt: row.get(5)?,
        transcript_slice: row.get(6)?,
        raw_response: row.get(7)?,
        parsed_title: row.get(8)?,
        parsed_activity: row.get(9)?,
        created_at: row.get(10)?,
    })
}

impl Store {
    /// Record one distill round-trip. Returns the assigned `id`.
    pub fn record_llm_call(&self, row: &NewLlmCall) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO llm_calls
                 (pubkey, window_hash, provider, model, system_prompt,
                  transcript_slice, raw_response, parsed_title, parsed_activity, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.pubkey,
                row.window_hash,
                row.provider,
                row.model,
                row.system_prompt,
                row.transcript_slice,
                row.raw_response,
                row.parsed_title,
                row.parsed_activity,
                row.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch one round-trip by id.
    pub fn get_llm_call(&self, id: i64) -> Result<Option<LlmCallRow>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLS} FROM llm_calls WHERE id=?1"),
                params![id],
                row_to_llm_call,
            )
            .optional()?)
    }

    /// Most recent round-trips for a session, newest first, capped at `limit`.
    pub fn latest_llm_calls_for_pubkey(&self, pubkey: &str, limit: u32) -> Result<Vec<LlmCallRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM llm_calls
             WHERE pubkey=?1
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![pubkey, limit], row_to_llm_call)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// All round-trips that fed the same transcript window, oldest first.
    pub fn llm_calls_by_window_hash(&self, window_hash: &str) -> Result<Vec<LlmCallRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM llm_calls
             WHERE window_hash=?1
             ORDER BY created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map(params![window_hash], row_to_llm_call)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// The round-trip for `pubkey` whose `created_at` is closest to
    /// `at_millis` — answers "what exact input drove the status at time T".
    pub fn find_llm_call_near(&self, pubkey: &str, at_millis: i64) -> Result<Option<LlmCallRow>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM llm_calls
                     WHERE pubkey=?1
                     ORDER BY ABS(created_at - ?2) ASC, created_at ASC LIMIT 1"
                ),
                params![pubkey, at_millis],
                row_to_llm_call,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{llm_calls::NewLlmCall, Store};

    fn call(pubkey: &str, window_hash: &str, created_at: i64) -> NewLlmCall {
        NewLlmCall {
            pubkey: pubkey.into(),
            window_hash: window_hash.into(),
            provider: "claude-cli".into(),
            model: "claude-haiku".into(),
            system_prompt: "SYSTEM".into(),
            transcript_slice: "TRANSCRIPT".into(),
            raw_response: "TITLE: Fix bug\nNOW: reading logs".into(),
            parsed_title: Some("Fix bug".into()),
            parsed_activity: Some("reading logs".into()),
            created_at,
        }
    }

    #[test]
    fn record_then_get_round_trips() {
        let s = Store::open_memory().unwrap();
        let id = s.record_llm_call(&call("sid-1", "hash-a", 1_000)).unwrap();

        let row = s.get_llm_call(id).unwrap().unwrap();
        assert_eq!(row.id, id);
        assert_eq!(row.pubkey, "sid-1");
        assert_eq!(row.window_hash, "hash-a");
        assert_eq!(row.provider, "claude-cli");
        assert_eq!(row.parsed_title.as_deref(), Some("Fix bug"));
        assert_eq!(row.parsed_activity.as_deref(), Some("reading logs"));
        assert_eq!(row.created_at, 1_000);
    }

    #[test]
    fn get_missing_id_returns_none() {
        let s = Store::open_memory().unwrap();
        assert!(s.get_llm_call(999).unwrap().is_none());
    }

    #[test]
    fn latest_for_session_orders_newest_first_and_respects_limit() {
        let s = Store::open_memory().unwrap();
        s.record_llm_call(&call("sid-1", "hash-a", 1_000)).unwrap();
        s.record_llm_call(&call("sid-1", "hash-b", 3_000)).unwrap();
        s.record_llm_call(&call("sid-1", "hash-c", 2_000)).unwrap();
        // Different session must not leak in.
        s.record_llm_call(&call("sid-2", "hash-d", 4_000)).unwrap();

        let rows = s.latest_llm_calls_for_pubkey("sid-1", 2).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].created_at, 3_000);
        assert_eq!(rows[1].created_at, 2_000);
    }

    #[test]
    fn by_window_hash_filters_and_orders_oldest_first() {
        let s = Store::open_memory().unwrap();
        s.record_llm_call(&call("sid-1", "hash-a", 2_000)).unwrap();
        s.record_llm_call(&call("sid-2", "hash-a", 1_000)).unwrap();
        s.record_llm_call(&call("sid-3", "hash-b", 500)).unwrap();

        let rows = s.llm_calls_by_window_hash("hash-a").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].created_at, 1_000);
        assert_eq!(rows[1].created_at, 2_000);
    }

    #[test]
    fn find_near_picks_closest_created_at() {
        let s = Store::open_memory().unwrap();
        s.record_llm_call(&call("sid-1", "hash-a", 1_000)).unwrap();
        s.record_llm_call(&call("sid-1", "hash-b", 5_000)).unwrap();
        s.record_llm_call(&call("sid-1", "hash-c", 9_000)).unwrap();

        let row = s.find_llm_call_near("sid-1", 6_000).unwrap().unwrap();
        assert_eq!(row.window_hash, "hash-b");

        // No rows for an unknown session.
        assert!(s.find_llm_call_near("sid-none", 6_000).unwrap().is_none());
    }
}
