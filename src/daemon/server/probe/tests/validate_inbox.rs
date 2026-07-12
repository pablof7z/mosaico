use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_inbox_reports_delivered_inbound_row() {
    let state = DaemonState::new_for_test().await;
    seed_session(&state, "s1");
    state
        .with_store(|s| {
            s.enqueue_inbox("evt-in", "s1", "pk-from", "room", "hello inbox", 100)?;
            s.mark_delivered("evt-in", "s1", 120)?;
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
            s.enqueue_inbox("evt-pending", "s1", "pk-from", "room", "pending body", 100)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "inbox/evt-pending/s1" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "inbox", "not_proven");
    assert_eq!(v["inbox_evidence"]["target_session"], "s1");
    assert_eq!(v["inbox_evidence"]["pending_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_inbox_reports_management_completion() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            assert!(s.claim_management_command("evt-mgmt", "pk-admin", "room", "restart", 100)?);
            s.complete_management_command("evt-mgmt", 120)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "inbox:evt-mgmt:management" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "inbox", "passed");
    assert_eq!(v["inbox_evidence"]["rows"][0]["target_kind"], "management");
    assert_eq!(v["inbox_evidence"]["delivered_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_inbox_reports_offline_mention_tombstone() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            assert!(s.claim_offline_mention(
                "evt-mention",
                "pk-agent",
                "pk-from",
                "room",
                "wake up",
                100,
            )?);
            s.complete_offline_mention("evt-mention", "pk-agent", 120)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "inbox:evt-mention" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "inbox", "passed");
    assert_eq!(
        v["inbox_evidence"]["rows"][0]["target_kind"],
        "offline_mention"
    );
    assert_eq!(v["inbox_evidence"]["delivered_count"], 1);
}

fn seed_session(state: &std::sync::Arc<DaemonState>, session_id: &str) {
    state
        .with_store(|s| {
            s.upsert_session_row(
                session_id,
                &RegisterSession {
                    harness: "codex".into(),
                    external_id_kind: "native".into(),
                    external_id: session_id.into(),
                    agent_slug: "agent".into(),
                    agent_pubkey: "pk-agent".into(),
                    channel_h: "room".into(),
                    transcript_path: None,
                    child_pid: None,
                    resume_id: String::new(),
                    now: 100,
                },
            )?;
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
