use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_inbox_reports_delivered_inbound_row() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state, "pk-agent");
    state
        .with_store(|s| {
            s.enqueue_inbox("evt-in", "pk-agent", "pk-from", "room", "hello inbox", 100)?;
            s.mark_delivered("evt-in", "pk-agent", 120)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "inbox:evt-in" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "inbox", "passed");
    assert_eq!(v["inbox_evidence"]["row_count"], 1);
    assert_eq!(v["inbox_evidence"]["delivered_count"], 1);
    assert_eq!(v["inbox_evidence"]["rows"][0]["session_alive"], true);
}

#[tokio::test]
async fn rpc_probe_validate_inbox_target_reports_pending_as_not_proven() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state, "s1");
    state
        .with_store(|s| {
            s.enqueue_inbox(
                "evt-pending",
                "pk-agent",
                "pk-from",
                "room",
                "pending body",
                100,
            )?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "inbox/evt-pending/pk-agent" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "inbox", "not_proven");
    assert_eq!(v["inbox_evidence"]["target_pubkey"], "pk-agent");
    assert_eq!(v["inbox_evidence"]["pending_count"], 1);
}

fn seed_session(state: &std::sync::Arc<DaemonState>, pubkey: &str) {
    state
        .with_store(|s| {
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                agent_slug: "agent".into(),
                pubkey: pubkey.into(),
                channel_h: "room".into(),
                transcript_path: None,
                child_pid: None,
                now: 100,
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
