use super::*;
use crate::instrument::{changed_summary_json, window_hash};
use crate::state::llm_calls::NewLlmCall;
use crate::state::receipts::NewReceipt;
use crate::state::{RecordMessage, RelayEvent};

#[tokio::test]
async fn rpc_probe_validate_event_accepts_chat_event_without_trellis_receipt() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-chat-123".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "chat evidence".into(),
                created_at: 110,
                direction: "outbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-chat-123".into()),
                error: None,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:event-chat" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "event", "passed");
    assert_check_status(&v, "explain", "not_proven");
    assert_eq!(v["event_evidence"]["message_found"], true);
    assert_eq!(v["event_evidence"]["receipt_count"], 0);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap().contains("no Trellis receipt")));
}

#[tokio::test]
async fn rpc_probe_validate_event_prefix_explains_trellis_artifact() {
    let state = DaemonState::new_for_test().await;
    let wh = window_hash("transcript slice");
    state
        .with_store(|s| {
            s.record_llm_call(&NewLlmCall {
                pubkey: "s1".into(),
                window_hash: wh.clone(),
                provider: "ollama".into(),
                model: "glm".into(),
                system_prompt: "system".into(),
                transcript_slice: "transcript slice".into(),
                raw_response: "TITLE: T\nNOW: A".into(),
                parsed_title: Some("T".into()),
                parsed_activity: Some("A".into()),
                created_at: 1_700_000_010,
            })?;
            s.record_receipt(&NewReceipt {
                surface: "status".into(),
                transaction_id: 7,
                revision: 3,
                changed_summary: changed_summary_json(&[], &[], &[], Some("s1"), Some(&wh)),
                commands: r#"[{"kind":"replace","key":"status/s1"}]"#.into(),
                artifact_ref: Some("evt-status-long".into()),
                created_at: 1_700_000_011,
            })?;
            let outbox = s.enqueue_outbox(r#"{"id":"evt-status-long"}"#, 1_700_000_011)?;
            s.apply_outbox_projection(outbox, "published", None, false)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:evt-status" }),
    )
    .unwrap();

    assert_eq!(v["surface"], "status");
    assert_check_status(&v, "event", "passed");
    assert_check_status(&v, "explain", "passed");
    assert_eq!(v["event_evidence"]["receipt_count"], 1);
    assert_eq!(v["event_evidence"]["outbox_store_count"], 1);
    assert_eq!(v["event_evidence"]["outbox_published"], true);
    assert_eq!(v["explain"]["llm_call"]["parsed_activity"], "A");
}

#[tokio::test]
async fn rpc_probe_validate_event_fails_failed_outbox_publish() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            let id = s.enqueue_outbox(r#"{"id":"evt-failed"}"#, 100)?;
            s.apply_outbox_projection(id, "failed", Some("relay rejected event"), true)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:evt-failed" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "event", "failed");
    assert_eq!(v["event_evidence"]["outbox_failed"], true);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l.as_str().unwrap() == "outbox row records a failed relay publish outcome"));
}

#[tokio::test]
async fn rpc_probe_validate_event_reports_raw_relay_context() {
    let state = DaemonState::new_for_test().await;
    seed_relay_chat(&state, "event-raw-123", "pk-author", r#"[["h","room"]]"#);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:event-raw" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "event", "passed");
    assert_eq!(v["event_evidence"]["relay_event_found"], true);
    assert_eq!(v["event_evidence"]["relay_tags_valid"], true);
    assert_eq!(v["event_evidence"]["relay_channel_found"], true);
    assert_eq!(v["event_evidence"]["relay_author_profile_found"], true);
    assert_eq!(v["event_evidence"]["relay_author_role"], "member");
}

#[tokio::test]
async fn rpc_probe_validate_event_fails_chat_author_outside_hydrated_roster() {
    let state = DaemonState::new_for_test().await;
    seed_relay_chat(
        &state,
        "event-nonmember-123",
        "pk-stranger",
        r#"[["h","room"]]"#,
    );

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:event-nonmember" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "event", "failed");
    assert_eq!(v["event_evidence"]["relay_author_member_found"], false);
    assert!(v["event_evidence"]["relay_validation_reason"]
        .as_str()
        .unwrap()
        .contains("membership snapshot"));
}

#[tokio::test]
async fn rpc_probe_validate_event_fails_invalid_relay_tags_json() {
    let state = DaemonState::new_for_test().await;
    seed_relay_chat(&state, "event-bad-tags-123", "pk-author", "not json");

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:event-bad-tags" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "event", "failed");
    assert_eq!(v["event_evidence"]["relay_tags_valid"], false);
    assert!(v["event_evidence"]["relay_validation_reason"]
        .as_str()
        .unwrap()
        .contains("tags_json"));
}

#[tokio::test]
async fn rpc_probe_validate_missing_event_is_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event:missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "event", "not_proven");
    assert_check_status(&v, "explain", "not_proven");
    assert_eq!(v["event_evidence"]["found"], false);
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

fn seed_relay_chat(state: &std::sync::Arc<DaemonState>, id: &str, pubkey: &str, tags_json: &str) {
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "", "", 100)?;
            s.replace_channel_admins("room", &Vec::<String>::new(), 101)?;
            s.replace_channel_members("room", &["pk-author".to_string()], 102)?;
            s.upsert_profile("pk-author", "Author", "author", "host", false, 103)?;
            s.insert_event(&RelayEvent {
                id: id.into(),
                kind: crate::fabric::nip29::wire::KIND_CHAT as u32,
                pubkey: pubkey.into(),
                created_at: 104,
                channel_h: "room".into(),
                d_tag: String::new(),
                content: "raw relay event".into(),
                tags_json: tags_json.into(),
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}
