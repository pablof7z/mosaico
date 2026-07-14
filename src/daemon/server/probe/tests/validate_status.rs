use super::*;
use crate::state::{RegisterSession, Status};

#[tokio::test]
async fn rpc_probe_validate_status_reports_published_graph_evidence() {
    let state = DaemonState::new_for_test().await;
    seed_status_graph(&state, "s1");

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "status:s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "status_outcome", "passed");
    assert_check_status(&v, "why", "passed");
    assert_check_status(&v, "state", "passed");
    assert_eq!(v["status_evidence"]["graph_found"], true);
    assert_eq!(v["status_evidence"]["session_row_found"], false);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("no local session row")));
}

#[tokio::test]
async fn rpc_probe_validate_status_fails_dead_local_session_contradiction() {
    let state = DaemonState::new_for_test().await;
    seed_status_graph(&state, "s1");
    state
        .with_store(|s| {
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: "s1".into(),
                agent_slug: "coder".into(),
                channel_h: "room".into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            s.mark_dead("s1")?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "status/s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "status_outcome", "failed");
    assert_eq!(v["status_evidence"]["graph_found"], true);
    assert_eq!(v["status_evidence"]["session_row_found"], true);
    assert_eq!(v["status_evidence"]["session_alive"], false);
    assert!(v["status_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("local session row is dead"));
}

#[tokio::test]
async fn rpc_probe_validate_status_accepts_live_relay_status_without_local_graph() {
    let state = DaemonState::new_for_test().await;
    seed_relay_status(&state, "s-peer", crate::util::now_secs() + 100);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "status:s-peer" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "status_outcome", "passed");
    assert_eq!(v["status_evidence"]["graph_found"], false);
    assert_eq!(v["status_evidence"]["relay_status_live"], true);
    assert_eq!(v["status_evidence"]["relay_live_channels"], json!(["room"]));
    assert!(v["limitations"].as_array().unwrap().iter().any(|l| {
        l.as_str()
            .unwrap()
            .contains("no local Trellis status graph")
    }));
}

#[tokio::test]
async fn rpc_probe_validate_status_reports_expired_relay_status_as_not_proven() {
    let state = DaemonState::new_for_test().await;
    seed_relay_status(&state, "s-peer", crate::util::now_secs().saturating_sub(1));

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "status:s-peer" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "status_outcome", "not_proven");
    assert_eq!(v["status_evidence"]["relay_status_found"], true);
    assert_eq!(v["status_evidence"]["relay_status_live"], false);
    assert_eq!(v["status_evidence"]["relay_expired_count"], 1);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("all are expired")));
}

fn seed_status_graph(state: &std::sync::Arc<DaemonState>, pubkey: &str) {
    state
        .status
        .lock()
        .unwrap()
        .on_session_started(
            pubkey,
            "laptop",
            "coder",
            ".",
            BTreeSet::from(["room".to_string()]),
            true,
            "T",
            "reading",
            100,
        )
        .unwrap();
}

fn seed_relay_status(state: &std::sync::Arc<DaemonState>, pubkey: &str, expiration: u64) {
    state
        .with_store(|s| {
            s.upsert_status(&Status {
                pubkey: pubkey.into(),
                channel_h: "room".into(),
                slug: "peer".into(),
                title: "Peer task".into(),
                activity: "checking fabric".into(),
                busy: true,
                last_seen: 200,
                updated_at: 190,
                expiration,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    assert_eq!(check_row(v, name)["status"], status);
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .expect("check row")
}
