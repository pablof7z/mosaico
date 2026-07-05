//! `trellis_commits` — the all-commit ledger (frontier design §4.1).
//!
//! Receipts (`state::receipts`) exist only for EFFECTFUL commits, so the value
//! evidence — suppressed publishes, no-op recomputes, unchanged hook frames — is
//! invisible. This store records EVERY transaction, effectful or not, so
//! `probe stats` can quantify "N commits, M suppressed, K no-ops" for a surface.
//!
//! Like `receipts`, this module is pure: no Trellis types cross it (the caller
//! flattens a `TransactionResult` into label arrays + counts via
//! `reconcile::labels::CommitFacts` before recording), and it never reads the
//! clock — the caller passes `created_at`. `transaction_id`/`revision` are stored
//! as `INTEGER` (Trellis's monotonic `u64` counters fit an `i64` column).

use super::*;

const COLS: &str = "id, surface, transaction_id, revision, mode, trigger_kind, trigger_ref, \
     changed_inputs_json, changed_derived_json, changed_collections_json, \
     resource_commands_json, output_frames_json, command_count, output_count, \
     effect_count, suppressed_count, noop, oracle_status, oracle_error, \
     duration_us, graph_nodes, graph_resources, created_at";

/// One persisted all-commit ledger row, flattened to plain fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
    pub id: i64,
    pub surface: String,
    pub transaction_id: i64,
    pub revision: i64,
    pub mode: String,
    pub trigger_kind: String,
    pub trigger_ref: String,
    pub changed_inputs_json: String,
    pub changed_derived_json: String,
    pub changed_collections_json: String,
    pub resource_commands_json: String,
    pub output_frames_json: String,
    pub command_count: i64,
    pub output_count: i64,
    pub effect_count: i64,
    pub suppressed_count: i64,
    pub noop: i64,
    pub oracle_status: Option<String>,
    pub oracle_error: Option<String>,
    pub duration_us: i64,
    pub graph_nodes: i64,
    pub graph_resources: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCommit {
    pub surface: String,
    pub transaction_id: i64,
    pub revision: i64,
    pub mode: String,
    pub trigger_kind: String,
    pub trigger_ref: String,
    pub changed_inputs_json: String,
    pub changed_derived_json: String,
    pub changed_collections_json: String,
    pub resource_commands_json: String,
    pub output_frames_json: String,
    pub command_count: i64,
    pub output_count: i64,
    pub effect_count: i64,
    pub suppressed_count: i64,
    pub noop: i64,
    pub oracle_status: Option<String>,
    pub oracle_error: Option<String>,
    pub duration_us: i64,
    pub graph_nodes: i64,
    pub graph_resources: i64,
    pub created_at: i64,
}

/// Aggregate value evidence for a surface over a window, powering `probe stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitStats {
    pub commits: i64,
    pub effectful: i64,
    pub noop: i64,
    pub command_count_sum: i64,
    pub output_count_sum: i64,
    pub effect_count_sum: i64,
    pub suppressed_count_sum: i64,
    pub duration_us_sum: i64,
    pub max_graph_nodes: i64,
    pub max_graph_resources: i64,
}

fn row_to_commit(row: &rusqlite::Row) -> rusqlite::Result<CommitRow> {
    Ok(CommitRow {
        id: row.get(0)?,
        surface: row.get(1)?,
        transaction_id: row.get(2)?,
        revision: row.get(3)?,
        mode: row.get(4)?,
        trigger_kind: row.get(5)?,
        trigger_ref: row.get(6)?,
        changed_inputs_json: row.get(7)?,
        changed_derived_json: row.get(8)?,
        changed_collections_json: row.get(9)?,
        resource_commands_json: row.get(10)?,
        output_frames_json: row.get(11)?,
        command_count: row.get(12)?,
        output_count: row.get(13)?,
        effect_count: row.get(14)?,
        suppressed_count: row.get(15)?,
        noop: row.get(16)?,
        oracle_status: row.get(17)?,
        oracle_error: row.get(18)?,
        duration_us: row.get(19)?,
        graph_nodes: row.get(20)?,
        graph_resources: row.get(21)?,
        created_at: row.get(22)?,
    })
}

impl Store {
    /// Record one flattened commit. Returns the assigned `id`.
    pub fn record_commit(&self, row: &NewCommit) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO trellis_commits
                 (surface, transaction_id, revision, mode, trigger_kind, trigger_ref,
                  changed_inputs_json, changed_derived_json, changed_collections_json,
                  resource_commands_json, output_frames_json,
                  command_count, output_count, effect_count, suppressed_count, noop,
                  oracle_status, oracle_error, duration_us, graph_nodes, graph_resources, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                row.surface,
                row.transaction_id,
                row.revision,
                row.mode,
                row.trigger_kind,
                row.trigger_ref,
                row.changed_inputs_json,
                row.changed_derived_json,
                row.changed_collections_json,
                row.resource_commands_json,
                row.output_frames_json,
                row.command_count,
                row.output_count,
                row.effect_count,
                row.suppressed_count,
                row.noop,
                row.oracle_status,
                row.oracle_error,
                row.duration_us,
                row.graph_nodes,
                row.graph_resources,
                row.created_at,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Most recent commits for a surface, newest first, capped at `limit`.
    pub fn latest_commits_for_surface(&self, surface: &str, limit: u32) -> Result<Vec<CommitRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLS} FROM trellis_commits
             WHERE surface=?1
             ORDER BY created_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![surface, limit], row_to_commit)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Aggregate value evidence for `surface` over commits with
    /// `created_at >= since`. Pure over the ledger — the proof `probe stats` works.
    pub fn commit_stats(&self, surface: &str, since: i64) -> Result<CommitStats> {
        Ok(self.conn.query_row(
            "SELECT
                 COUNT(*),
                 COALESCE(SUM(CASE WHEN noop=0 THEN 1 ELSE 0 END), 0),
                 COALESCE(SUM(noop), 0),
                 COALESCE(SUM(command_count), 0),
                 COALESCE(SUM(output_count), 0),
                 COALESCE(SUM(effect_count), 0),
                 COALESCE(SUM(suppressed_count), 0),
                 COALESCE(SUM(duration_us), 0),
                 COALESCE(MAX(graph_nodes), 0),
                 COALESCE(MAX(graph_resources), 0)
             FROM trellis_commits
             WHERE surface=?1 AND created_at >= ?2",
            params![surface, since],
            |r| {
                Ok(CommitStats {
                    commits: r.get(0)?,
                    effectful: r.get(1)?,
                    noop: r.get(2)?,
                    command_count_sum: r.get(3)?,
                    output_count_sum: r.get(4)?,
                    effect_count_sum: r.get(5)?,
                    suppressed_count_sum: r.get(6)?,
                    duration_us_sum: r.get(7)?,
                    max_graph_nodes: r.get(8)?,
                    max_graph_resources: r.get(9)?,
                })
            },
        )?)
    }
}

#[cfg(test)]
mod tests {
    use crate::state::{trellis_commits::NewCommit, Store};

    fn commit(surface: &str, noop: i64, commands: i64, created_at: i64) -> NewCommit {
        NewCommit {
            surface: surface.into(),
            transaction_id: 42,
            revision: 7,
            mode: "authoritative".into(),
            trigger_kind: "tick".into(),
            trigger_ref: "s1".into(),
            changed_inputs_json: r#"["status/s1/activity"]"#.into(),
            changed_derived_json: r#"["status/s1/content"]"#.into(),
            changed_collections_json: "[]".into(),
            resource_commands_json: "[]".into(),
            output_frames_json: "[]".into(),
            command_count: commands,
            output_count: 0,
            effect_count: commands,
            suppressed_count: noop,
            noop,
            oracle_status: None,
            oracle_error: None,
            duration_us: 250,
            graph_nodes: 6,
            graph_resources: 2,
            created_at,
        }
    }

    #[test]
    fn record_then_latest_orders_newest_first_and_filters_surface() {
        let s = Store::open_memory().unwrap();
        s.record_commit(&commit("status", 0, 1, 1_000)).unwrap();
        s.record_commit(&commit("status", 1, 0, 3_000)).unwrap();
        s.record_commit(&commit("status", 0, 2, 2_000)).unwrap();
        // A different surface must not leak in.
        s.record_commit(&commit("subscriptions", 0, 1, 4_000))
            .unwrap();

        let rows = s.latest_commits_for_surface("status", 10).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].created_at, 3_000);
        assert_eq!(rows[0].noop, 1);
        assert_eq!(rows[0].mode, "authoritative");
        assert_eq!(rows[0].trigger_ref, "s1");
        assert_eq!(rows[0].suppressed_count, 1);
        assert_eq!(rows[0].graph_resources, 2);
        assert_eq!(rows[2].created_at, 1_000);
        assert_eq!(rows[0].changed_inputs_json, r#"["status/s1/activity"]"#);
    }

    #[test]
    fn stats_aggregate_effectful_and_noop() {
        let s = Store::open_memory().unwrap();
        // Two effectful (1 + 2 commands) and one no-op, all within the window.
        s.record_commit(&commit("status", 0, 1, 1_000)).unwrap();
        s.record_commit(&commit("status", 1, 0, 2_000)).unwrap();
        s.record_commit(&commit("status", 0, 2, 3_000)).unwrap();
        // Out-of-window row is excluded by `since`.
        s.record_commit(&commit("status", 0, 5, 500)).unwrap();

        let stats = s.commit_stats("status", 1_000).unwrap();
        assert_eq!(stats.commits, 3);
        assert_eq!(stats.effectful, 2);
        assert_eq!(stats.noop, 1);
        assert_eq!(stats.command_count_sum, 3);
        assert_eq!(stats.effect_count_sum, 3);
        assert_eq!(stats.suppressed_count_sum, 1);
        assert_eq!(stats.max_graph_nodes, 6);
        assert_eq!(stats.max_graph_resources, 2);
        assert_eq!(stats.duration_us_sum, 750);
    }

    #[test]
    fn stats_over_empty_surface_is_zeroed() {
        let s = Store::open_memory().unwrap();
        let stats = s.commit_stats("hook_context", 0).unwrap();
        assert_eq!(stats.commits, 0);
        assert_eq!(stats.effectful, 0);
        assert_eq!(stats.max_graph_nodes, 0);
    }
}
