use super::*;
use crate::state::RecordMessage;

#[tokio::test]
async fn rpc_probe_validate_message_reports_read_model_and_delivery_edges() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.record_message(&record("event-123", "accepted", None))?;
            s.add_message_recipient("event-123", "pk-recipient", Some(120))?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "msg:event-123" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "message", "passed");
    assert_eq!(v["message_evidence"]["found"], true);
    assert_eq!(v["message_evidence"]["message_id"], "event-123");
    assert_eq!(v["message_evidence"]["channel_confirmed"], true);
    assert_eq!(v["message_evidence"]["recipient_count"], 1);
    assert_eq!(v["message_evidence"]["delivered_recipient_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_message_reports_missing_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "message/missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "message", "not_proven");
    assert_eq!(v["message_evidence"]["found"], false);
    assert!(v["message_evidence"]["summary"]
        .as_str()
        .unwrap()
        .contains("not in the canonical channel read model"));
}

#[tokio::test]
async fn rpc_probe_validate_message_reports_failed_sync_state() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_message(&record("event-failed", "failed", Some("relay rejected")))?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "message:event-failed" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "message", "failed");
    assert_eq!(v["message_evidence"]["sync_state"], "failed");
    assert_eq!(v["message_evidence"]["error"], "relay rejected");
}

fn record(id: &str, sync_state: &str, error: Option<&str>) -> RecordMessage {
    RecordMessage {
        message_id: id.to_string(),
        thread_id: "room".to_string(),
        channel_h: "room".to_string(),
        author_pubkey: "pk-author".to_string(),
        body: "hello from the fabric".to_string(),
        created_at: 110,
        direction: "outbound".to_string(),
        sync_state: sync_state.to_string(),
        native_event_id: Some(id.to_string()),
        error: error.map(str::to_string),
    }
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
