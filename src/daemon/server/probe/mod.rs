//! `probe` RPC arm — the hidden diagnostic surface over the reconciler frontier
//! (frontier design §4). The CLI (`tenex-edge probe <verb>`) forwards a `verb`
//! plus verb params here; this module dispatches to one implementation file per
//! verb, each kept small and independently testable:
//!
//! * `stats`    — aggregate value evidence over the all-commit ledger (§4.1).
//! * `oracle`   — the incremental-equals-full correctness check, live (§4.6).
//! * `simulate` — dry-run a fact via `tx.preview()`; the keystone (§3).
//! * `why`      — live causality for a `sub:`/`status:` handle (§4.3).
//! * `state`    — live values per surface: owners/refcounts, status inputs (§4.3).

use super::DaemonState;
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;

mod oracle;
mod simulate;
mod state;
mod stats;
mod why;

/// The reconciler surfaces the ledger records; `stats` with no `--surface`
/// reports all of them.
pub(super) const SURFACES: [&str; 3] = ["status", "subscriptions", "hook_context"];

/// Route a `probe` RPC to its verb. `params` carries `{"verb": <str>, ...}`.
pub(in crate::daemon::server) fn rpc_probe(
    state: &Arc<DaemonState>,
    params: &Value,
) -> Result<Value> {
    let verb = params
        .get("verb")
        .and_then(Value::as_str)
        .context("probe: missing `verb` param")?;
    match verb {
        "stats" => {
            let surface = params.get("surface").and_then(Value::as_str);
            let since = params.get("since").and_then(Value::as_i64).unwrap_or(0);
            state.with_store(|s| stats::stats_value(s, surface, since))
        }
        "oracle" => Ok(oracle::oracle_value(state)),
        "simulate" => simulate::simulate_value(state, params),
        "why" => why::why_value(state, params),
        "state" => state::state_value(state, params),
        other => Err(anyhow::anyhow!("probe: unknown verb `{other}`")),
    }
}

/// Shared param helper: a required string field.
fn required_str<'a>(params: &'a Value, key: &str) -> Result<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .with_context(|| format!("probe: missing `{key}` param"))
}

/// The surfaces the daemon holds as LIVE, long-lived graphs (oracle/why/state can
/// inspect them). `hook_context` is deliberately absent — it is rebuilt per render
/// (advisory), not a daemon-held graph.
fn not_live_note() -> Value {
    json!({
        "surface": "hook_context",
        "live_graph": false,
        "note": "advisory — rebuilt per render, not a daemon-held graph",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::CoverageSnapshot;
    use crate::state::trellis_commits::NewCommit;
    use std::collections::{BTreeMap, BTreeSet};

    /// End-to-end proof that the `probe` RPC — the lock/param/dispatch inch in
    /// `rpc_probe` — actually works over a REAL `DaemonState`, not merely that the
    /// pure value-fns compile. Builds a minimal offline state, DRIVES its live
    /// reconcilers (a session + a distill, one synced subscription) and its ledger
    /// (one recorded status commit), then calls `rpc_probe` for EVERY verb and
    /// asserts the returned JSON is well-formed and reflects the driven state.
    #[tokio::test]
    async fn rpc_probe_reflects_driven_state_for_every_verb() {
        let state = DaemonState::new_for_test().await;

        // Drive status: start a session, then a distill changing its activity.
        {
            let mut r = state.status.lock().expect("status mutex");
            r.on_session_started(
                "s1",
                "laptop",
                "coder",
                "pk1",
                ".",
                BTreeSet::from(["room".to_string()]),
                true,
                "T",
                "reading",
                1_700_000_010,
            )
            .unwrap();
            r.on_distill("s1", "T", "reviewing the PR", 1_700_000_010)
                .unwrap();
        }
        // Drive subscriptions: one session covering one channel.
        {
            let mut r = state.subs.lock().expect("subs mutex");
            let mut sessions = BTreeMap::new();
            sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
            r.sync(&CoverageSnapshot {
                daemon_channels: BTreeSet::from(["room".to_string()]),
                addressed_pubkeys: BTreeSet::new(),
                archived_channels: BTreeSet::new(),
                sessions,
            })
            .unwrap();
        }
        // Drive the ledger: one recorded status commit so `stats` counts it.
        state.with_store(|s| {
            s.record_commit(&NewCommit {
                surface: "status".into(),
                transaction_id: 1,
                revision: 1,
                mode: "authoritative".into(),
                trigger_kind: "distill".into(),
                trigger_ref: "s1".into(),
                changed_inputs_json: "[]".into(),
                changed_derived_json: "[]".into(),
                changed_collections_json: "[]".into(),
                resource_commands_json: "[]".into(),
                output_frames_json: "[]".into(),
                command_count: 1,
                output_count: 0,
                effect_count: 1,
                suppressed_count: 0,
                noop: 0,
                oracle_status: None,
                oracle_error: None,
                duration_us: 100,
                graph_nodes: 6,
                graph_resources: 0,
                created_at: 1_700_000_010,
            })
            .unwrap();
        });

        // oracle → both live surfaces green.
        let oracle = rpc_probe(&state, &json!({ "verb": "oracle" })).unwrap();
        assert_eq!(oracle["ok"], true);
        let ostatus = oracle["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["surface"] == "status")
            .expect("status surface row");
        assert_eq!(ostatus["status"], "green");

        // stats → the recorded status commit is counted as effectful.
        let stats = rpc_probe(&state, &json!({ "verb": "stats", "since": 0 })).unwrap();
        let sstatus = stats["surfaces"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["surface"] == "status")
            .expect("status stats row");
        assert_eq!(sstatus["commits"], 1);
        assert_eq!(sstatus["effectful"], 1);

        // simulate → a NEW activity would_publish (Replace) and the live graph is
        // untouched (revision unchanged).
        let sim = rpc_probe(
            &state,
            &json!({
                "verb": "simulate", "surface": "status", "session": "s1",
                "activity": "compiling", "title": null, "now": null,
            }),
        )
        .unwrap();
        assert_eq!(sim["would_publish"], true);
        assert_eq!(sim["commands"][0]["op"], "Replace");
        assert_eq!(sim["revision_before"], sim["revision_after"]);

        // why → the driven session's last command, attributed to the activity input.
        let why = rpc_probe(&state, &json!({ "verb": "why", "handle": "status:s1" })).unwrap();
        assert_eq!(why["found"], true);
        assert_eq!(why["last_kind"], "Replace");
        assert!(why["input_causes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l == "status/s1/activity"));

        // state → status rows carry the driven content; subs rows carry the channel.
        let st = rpc_probe(&state, &json!({ "verb": "state", "surface": "status" })).unwrap();
        let rows = st["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["session"], "s1");
        assert_eq!(rows[0]["activity"], "reviewing the PR");

        let subs = rpc_probe(
            &state,
            &json!({ "verb": "state", "surface": "subscriptions" }),
        )
        .unwrap();
        assert!(subs["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["resource_key"] == "sub/h/room"));
    }
}
