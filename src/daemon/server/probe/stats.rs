//! `probe stats`: aggregate value evidence per surface over the all-commit ledger
//! (§4.1). Pure over the [`Store`], so it is unit-testable without a live daemon.

use crate::state::Store;
use anyhow::Result;
use serde_json::{json, Value};

use super::SURFACES;

/// Build the per-surface stats value: commits, effectful vs suppressed no-ops,
/// command/output totals, latency, and the graph-size high-water mark.
pub(super) fn stats_value(s: &Store, surface: Option<&str>, since: i64) -> Result<Value> {
    let surfaces: Vec<&str> = match surface {
        Some(one) => vec![one],
        None => SURFACES.to_vec(),
    };
    let mut rows = Vec::with_capacity(surfaces.len());
    for surf in surfaces {
        let st = s.commit_stats(surf, since)?;
        rows.push(json!({
            "surface": surf,
            "commits": st.commits,
            "effectful": st.effectful,
            "noop": st.noop,
            "command_count_sum": st.command_count_sum,
            "output_count_sum": st.output_count_sum,
            "effect_count_sum": st.effect_count_sum,
            "suppressed_count_sum": st.suppressed_count_sum,
            "duration_us_sum": st.duration_us_sum,
            "max_graph_nodes": st.max_graph_nodes,
            "max_graph_resources": st.max_graph_resources,
            "latest_graph_resources": st.latest_graph_resources,
            "open_count": st.open_count,
            "close_count": st.close_count,
            "replace_count": st.replace_count,
            "refresh_count": st.refresh_count,
            "live_resource_balance": st.live_resource_balance,
            "resource_drift": st.resource_drift,
            "hook_unchanged_frames": st.hook_unchanged_frames,
            "duration_histogram": histogram(&st.duration_histogram),
            "graph_nodes_histogram": histogram(&st.graph_nodes_histogram),
            "graph_resources_histogram": histogram(&st.graph_resources_histogram),
            "latest_oracle_status": st.latest_oracle_status,
            "latest_oracle_error": st.latest_oracle_error,
        }));
    }
    Ok(json!({ "verb": "stats", "since": since, "surfaces": rows }))
}

pub(super) fn doctor_summary_value(s: &Store, since: i64) -> Result<Value> {
    let mut rows = Vec::new();
    for surf in SURFACES {
        let st = s.commit_stats(surf, since)?;
        rows.push(json!({
            "surface": surf,
            "mode": surface_mode(surf),
            "oracle_status": st.latest_oracle_status.unwrap_or_else(|| "unknown".into()),
            "oracle_error": st.latest_oracle_error,
            "suppressed_count": st.suppressed_count_sum,
            "hook_unchanged_frames": st.hook_unchanged_frames,
            "commits": st.commits,
        }));
    }
    Ok(json!({ "since": since, "surfaces": rows }))
}

fn surface_mode(surface: &str) -> &'static str {
    match surface {
        "status" | "subscriptions" | "hook_context" | "turn_lifecycle" | "cursor" | "outbox" => {
            "authoritative"
        }
        _ => "imperative",
    }
}

fn histogram(buckets: &[crate::state::trellis_commits::HistogramBucket]) -> Vec<serde_json::Value> {
    buckets
        .iter()
        .map(|b| json!({ "bucket": b.bucket, "count": b.count }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::trellis_commits::NewCommit;

    fn seed(
        s: &Store,
        surface: &str,
        noop: i64,
        commands: i64,
        command_json: &str,
        frame_json: &str,
        at: i64,
    ) {
        s.record_commit(&NewCommit {
            surface: surface.into(),
            transaction_id: 1,
            revision: 1,
            mode: "authoritative".into(),
            trigger_kind: "tick".into(),
            trigger_ref: "test".into(),
            changed_inputs_json: "[]".into(),
            changed_derived_json: "[]".into(),
            changed_collections_json: "[]".into(),
            resource_commands_json: command_json.into(),
            output_frames_json: frame_json.into(),
            command_count: commands,
            output_count: 0,
            effect_count: commands,
            suppressed_count: noop,
            noop,
            oracle_status: None,
            oracle_error: None,
            duration_us: 100,
            graph_nodes: 4,
            graph_resources: 2,
            created_at: at,
        })
        .unwrap();
    }

    #[test]
    fn stats_over_seeded_ledger_reports_per_surface_evidence() {
        let s = Store::open_memory().unwrap();
        // status: 2 effectful (3 commands total) + 1 suppressed no-op.
        seed(&s, "status", 0, 1, r#"[{"kind":"open"}]"#, "[]", 1_000);
        seed(&s, "status", 0, 2, r#"[{"kind":"refresh"}]"#, "[]", 2_000);
        seed(&s, "status", 1, 0, "[]", "[]", 3_000);
        // subscriptions: one effectful.
        seed(
            &s,
            "subscriptions",
            0,
            1,
            r#"[{"kind":"open"},{"kind":"close"}]"#,
            "[]",
            1_500,
        );
        seed(
            &s,
            "hook_context",
            1,
            0,
            "[]",
            r#"[{"kind":"unchanged"}]"#,
            2_500,
        );

        let v = stats_value(&s, None, 0).unwrap();
        assert_eq!(v["verb"], "stats");
        let surfaces = v["surfaces"].as_array().unwrap();
        assert_eq!(surfaces.len(), 6);

        let status = surfaces.iter().find(|r| r["surface"] == "status").unwrap();
        assert_eq!(status["commits"], 3);
        assert_eq!(status["effectful"], 2);
        assert_eq!(status["noop"], 1);
        assert_eq!(status["command_count_sum"], 3);
        assert_eq!(status["effect_count_sum"], 3);
        assert_eq!(status["suppressed_count_sum"], 1);
        assert_eq!(status["open_count"], 1);
        assert_eq!(status["refresh_count"], 1);
        assert_eq!(status["max_graph_nodes"], 4);
        assert_eq!(status["max_graph_resources"], 2);

        let hook = surfaces
            .iter()
            .find(|r| r["surface"] == "hook_context")
            .unwrap();
        assert_eq!(hook["commits"], 1);
        assert_eq!(hook["hook_unchanged_frames"], 1);

        let cursor = surfaces.iter().find(|r| r["surface"] == "cursor").unwrap();
        assert_eq!(cursor["commits"], 0);
        let outbox = surfaces.iter().find(|r| r["surface"] == "outbox").unwrap();
        assert_eq!(outbox["commits"], 0);
    }

    #[test]
    fn stats_single_surface_and_since_window() {
        let s = Store::open_memory().unwrap();
        seed(&s, "status", 0, 1, "[]", "[]", 500);
        seed(&s, "status", 0, 1, "[]", "[]", 2_000);

        let v = stats_value(&s, Some("status"), 1_000).unwrap();
        let surfaces = v["surfaces"].as_array().unwrap();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0]["commits"], 1);
    }
}
