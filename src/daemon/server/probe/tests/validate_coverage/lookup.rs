use super::*;
use crate::state::RecordMessage;
use nostr_sdk::prelude::{EventId, PublicKey, ToBech32};

#[tokio::test]
async fn rpc_probe_validate_lookup_finds_durable_handles() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-lookup".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-lookup".into()),
                error: None,
            })?;
            s.add_message_recipient("event-lookup", "pk-recipient", None)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "lookup:event-lookup" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "lookup", "passed");
    assert_eq!(v["coverage_evidence"]["kind"], "validation_lookup");
    assert_eq!(v["coverage_evidence"]["found"], true);
    assert_lookup_target(&v, "message:event-lookup");
    assert_lookup_target(&v, "recipient:event-lookup:pk-recipient");
}

#[tokio::test]
async fn rpc_probe_validate_bare_identifier_uses_lookup() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-lookup".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-lookup".into()),
                error: None,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "event-lookup" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "lookup", "passed");
    assert_eq!(v["coverage_evidence"]["needle"], "event-lookup");
    assert_lookup_target(&v, "message:event-lookup");
}

#[tokio::test]
async fn rpc_probe_validate_npub_lookup_normalizes_to_hex_pubkey() {
    let state = DaemonState::new_for_test().await;
    let pubkey = "379e863e8357163b5bce5d2688dc4f1dcc2d505222fb8d74db600f30535dfdfe";
    let npub = PublicKey::from_hex(pubkey).unwrap().to_bech32().unwrap();
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-npub-lookup".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: pubkey.into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-npub-lookup".into()),
                error: None,
            })?;
            s.upsert_profile(pubkey, "coder", "coder", "host", false, 100)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let bare = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("nostr:{npub}") }),
    )
    .unwrap();
    assert_check_status(&bare, "lookup", "passed");
    assert_eq!(bare["coverage_evidence"]["needle"], pubkey);
    assert_lookup_target(&bare, "message:event-npub-lookup");
    assert_lookup_target(&bare, &format!("profile:{pubkey}"));

    let explicit = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("lookup:{npub}") }),
    )
    .unwrap();
    assert_check_status(&explicit, "lookup", "passed");
    assert_eq!(explicit["coverage_evidence"]["needle"], pubkey);
}

#[tokio::test]
async fn rpc_probe_validate_note_lookup_normalizes_to_hex_event_id() {
    let state = DaemonState::new_for_test().await;
    let event_id = "2be17aa3031bdcb006f0fce80c146dea9c1c0268b0af2398bb673365c6444d45";
    let note = EventId::from_hex(event_id).unwrap().to_bech32().unwrap();
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: event_id.into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some(event_id.into()),
                error: None,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": format!("lookup:nostr:{note}") }),
    )
    .unwrap();

    assert_check_status(&v, "lookup", "passed");
    assert_eq!(v["coverage_evidence"]["needle"], event_id);
    assert_lookup_target(&v, &format!("message:{event_id}"));
}

#[tokio::test]
async fn rpc_probe_validate_lookup_reports_absence_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "find:no-such-identifier" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert_check_status(&v, "lookup", "not_proven");
    assert_eq!(v["coverage_evidence"]["found"], false);
}

fn assert_lookup_target(v: &serde_json::Value, target: &str) {
    assert!(v["coverage_evidence"]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["target"].as_str() == Some(target)));
}
