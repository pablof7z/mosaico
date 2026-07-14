use super::*;
use serde_json::json;
use std::collections::BTreeSet;

#[tokio::test]
async fn rpc_probe_stats_quantifies_shared_subscription_beachhead() {
    let state = DaemonState::new_for_test().await;

    drive_subscription_sync_for_stats(&state, subscription_snapshot(&["s1", "s2"]), 1_000);
    drive_subscription_sync_for_stats(&state, subscription_snapshot(&["s2"]), 2_000);

    let after_first_owner = rpc_probe(
        &state,
        &json!({ "verb": "stats", "surface": "subscriptions", "since": 0 }),
    )
    .unwrap();
    let first_row = &after_first_owner["surfaces"][0];
    assert_eq!(first_row["open_count"], 3);
    assert_eq!(first_row["close_count"], 0);
    assert_eq!(first_row["latest_graph_resources"], 3);
    assert_eq!(first_row["resource_drift"], false);

    drive_subscription_sync_for_stats(&state, subscription_snapshot(&[]), 3_000);

    let final_stats = rpc_probe(
        &state,
        &json!({ "verb": "stats", "surface": "subscriptions", "since": 0 }),
    )
    .unwrap();
    let row = &final_stats["surfaces"][0];
    assert_eq!(row["open_count"], 3);
    assert_eq!(row["close_count"], 2);
    assert_eq!(row["live_resource_balance"], 1);
    assert_eq!(row["latest_graph_resources"], 1);
    assert_eq!(row["resource_drift"], false);
}

fn drive_subscription_sync_for_stats(
    state: &std::sync::Arc<DaemonState>,
    snapshot: CoverageSnapshot,
    created_at: i64,
) {
    let facts = {
        let mut rec = state.subs.lock().unwrap();
        let (_effects, result) = rec.sync(&snapshot).unwrap();
        let mut facts = crate::reconcile::CommitFacts::from_result(
            rec.labels(),
            &result,
            rec.graph_node_count(),
        );
        facts.graph_resources = rec.state_rows().len() as i64;
        facts
    };
    state.with_store(|s| {
        crate::instrument::record_commit(s, "subscriptions", "sync", None, &facts, 0, created_at)
    });
}

fn subscription_snapshot(pubkeys: &[&str]) -> CoverageSnapshot {
    let sessions = pubkeys
        .iter()
        .map(|id| ((*id).to_string(), BTreeSet::from(["room".to_string()])))
        .collect();
    CoverageSnapshot {
        daemon_channels: BTreeSet::new(),
        addressed_pubkeys: BTreeSet::new(),
        archived_channels: BTreeSet::new(),
        sessions,
    }
}
