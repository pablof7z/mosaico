//! `probe oracle` (§4.6): run the incremental-equals-full check on each
//! daemon-held reconciler graph, live, under its lock. This proves the graph's
//! own bookkeeping is self-consistent — NOT that the host effects it drove are
//! correct — so the render is scrupulously honest about what was and was not
//! proven, and names the uncovered (imperative) surfaces.

use super::{not_live_note, DaemonState};
use crate::reconcile::frontier;
use serde_json::{json, Value};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::daemon::server) struct OracleSurface {
    pub surface: &'static str,
    pub status: &'static str,
    pub error: Option<String>,
    pub revision: u64,
    pub nodes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::daemon::server) struct OracleReport {
    pub ok: bool,
    pub surfaces: Vec<OracleSurface>,
}

pub(in crate::daemon::server) fn oracle_report(state: &Arc<DaemonState>) -> OracleReport {
    let status = {
        let r = state.status.lock().expect("status mutex poisoned");
        r.clone()
    };
    let subs = {
        let r = state.subs.lock().expect("subs mutex poisoned");
        r.clone()
    };

    let mut ok = true;
    let status_row = check(
        "status",
        status.assert_oracle(),
        status.revision(),
        status.graph_node_count(),
    );
    ok &= status_row.status == "green";
    let subs_row = check(
        "subscriptions",
        subs.assert_oracle(),
        subs.revision(),
        subs.graph_node_count(),
    );
    ok &= subs_row.status == "green";

    OracleReport {
        ok,
        surfaces: vec![status_row, subs_row],
    }
}

pub(super) fn oracle_value(state: &Arc<DaemonState>) -> Value {
    let report = oracle_report(state);
    let mut surfaces = report
        .surfaces
        .iter()
        .map(surface_value)
        .collect::<Vec<_>>();
    surfaces.push(not_live_note());

    json!({
        "verb": "oracle",
        "ok": report.ok,
        "oracle": if report.ok { "green" } else { "red" },
        "surfaces": surfaces,
        // The load-bearing honesty (§4.6, §8): a green oracle is graph-bookkeeping
        // correctness, not host-effect correctness.
        "surface_correctness_proven": false,
        "surface_correctness": "NOT PROVEN",
        "host_seam_coverage_percent": frontier::host_seam_coverage_percent(),
        "covered": ["status", "subscriptions"],
        "uncovered": frontier::uncovered_bypass_risks(),
    })
}

fn check(
    surface: &'static str,
    outcome: trellis_core::GraphResult<()>,
    revision: u64,
    nodes: usize,
) -> OracleSurface {
    match outcome {
        Ok(()) => OracleSurface {
            surface,
            status: "green",
            error: None,
            revision,
            nodes,
        },
        Err(e) => OracleSurface {
            surface,
            status: "red",
            error: Some(format!("{e}")),
            revision,
            nodes,
        },
    }
}

fn surface_value(surface: &OracleSurface) -> Value {
    json!({
        "surface": surface.surface,
        "live_graph": true,
        "status": surface.status,
        "revision": surface.revision,
        "nodes": surface.nodes,
        "error": surface.error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly-driven daemon reconciler reports green for both live surfaces and
    /// prints the honest uncovered list. Built directly (no live daemon) by driving
    /// the same reconcilers the daemon holds.
    #[test]
    fn freshly_driven_reconcilers_report_green() {
        use crate::reconcile::{CoverageSnapshot, StatusReconciler, SubscriptionReconciler};
        use std::collections::{BTreeMap, BTreeSet};

        let mut status = StatusReconciler::new(90, 30);
        status
            .on_session_started(
                "s1",
                "laptop",
                "coder",
                "pk1",
                ".",
                BTreeSet::from(["room".to_string()]),
                true,
                "T",
                "reading",
                100,
            )
            .unwrap();

        let mut subs = SubscriptionReconciler::new().unwrap();
        let mut sessions = BTreeMap::new();
        sessions.insert("s1".to_string(), BTreeSet::from(["room".to_string()]));
        subs.sync(&CoverageSnapshot {
            daemon_channels: BTreeSet::from(["room".to_string()]),
            addressed_pubkeys: BTreeSet::new(),
            archived_channels: BTreeSet::new(),
            sessions,
        })
        .unwrap();

        // Mirror what oracle_value does per surface.
        let status_row = check(
            "status",
            status.assert_oracle(),
            status.revision(),
            status.graph_node_count(),
        );
        let subs_row = check(
            "subscriptions",
            subs.assert_oracle(),
            subs.revision(),
            subs.graph_node_count(),
        );

        assert_eq!(status_row.status, "green");
        assert_eq!(subs_row.status, "green");
        assert!(status_row.nodes > 0);
    }
}
