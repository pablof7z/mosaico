use super::*;
use crate::state::RecordMessage;

#[tokio::test]
async fn rpc_probe_validate_recipient_reports_delivered_edge() {
    let state = DaemonState::new_for_test().await;
    seed_message(&state, "event-123", "accepted", None);
    state
        .with_store(|s| s.add_message_recipient("event-123", "pk-recipient", Some(120)))
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "recipient:event-123:pk-recipient" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "recipient", "passed");
    assert_eq!(v["recipient_evidence"]["message_id"], "event-123");
    assert_eq!(v["recipient_evidence"]["found"], true);
    assert_eq!(v["recipient_evidence"]["delivered"], true);
    assert_eq!(v["recipient_evidence"]["matching_row_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_recipient_reports_pending_edge_as_not_proven() {
    let state = DaemonState::new_for_test().await;
    seed_message(&state, "event-pending", "accepted", None);
    state
        .with_store(|s| s.add_message_recipient("event-pending", "pk-recipient", None))
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "delivery/event-pending/pk-recipient" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "recipient", "not_proven");
    assert_eq!(v["recipient_evidence"]["found"], true);
    assert_eq!(v["recipient_evidence"]["delivered"], false);
    assert_eq!(v["recipient_evidence"]["pending"], true);
}

#[tokio::test]
async fn rpc_probe_validate_recipient_fails_when_hydrated_set_excludes_pubkey() {
    let state = DaemonState::new_for_test().await;
    seed_message(&state, "event-other", "accepted", None);
    state
        .with_store(|s| s.add_message_recipient("event-other", "pk-other", Some(130)))
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "recipient:event-other:pk-recipient" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "recipient", "failed");
    assert_eq!(v["recipient_evidence"]["found"], false);
    assert_eq!(v["recipient_evidence"]["recipient_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_recipient_reports_missing_message_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "recipient:missing:pk-recipient" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "recipient", "not_proven");
    assert_eq!(v["recipient_evidence"]["message_found"], false);
    assert!(v["recipient_evidence"]["summary"]
        .as_str()
        .unwrap()
        .contains("not in the canonical channel read model"));
}

fn seed_message(state: &DaemonState, id: &str, sync_state: &str, error: Option<&str>) {
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.record_message(&RecordMessage {
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
