use super::*;
use crate::state::RecordMessage;

mod lookup;

#[tokio::test]
async fn rpc_probe_validate_coverage_maps_live_durable_tables() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "coverage" })).unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "validation_coverage", "passed");
    assert_eq!(v["coverage_evidence"]["coverage_ok"], true);
    assert_eq!(v["coverage_evidence"]["uncovered_tables"], json!([]));
    assert_eq!(
        v["coverage_evidence"]["covered_table_count"],
        v["coverage_evidence"]["table_count"]
    );
    assert_has_table_target(&v, "trellis_replay_capsules", "capsule:<id>", "direct");
    assert_has_surface_mode(&v, "status", "authoritative");
    assert_has_surface_mode(&v, "session_start", "advisory");
}

#[tokio::test]
async fn rpc_probe_validate_coverage_supports_alias_target() {
    let state = DaemonState::new_for_test().await;

    for target in ["validation_coverage", "inventory"] {
        let v = rpc_probe(&state, &json!({ "verb": "validate", "target": target })).unwrap();

        assert_check_status(&v, "validation_coverage", "passed");
        assert_eq!(v["coverage_evidence"]["target"], target);
    }
}

#[tokio::test]
async fn rpc_probe_validate_table_reports_profile_and_target_family() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-123".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-123".into()),
                error: None,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "table:messages" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "table_coverage", "passed");
    assert_eq!(v["coverage_evidence"]["kind"], "validation_table");
    assert_eq!(v["verdict"], "passed");
    assert_eq!(v["coverage_evidence"]["table"], "messages");
    assert_eq!(v["coverage_evidence"]["present"], true);
    assert_eq!(v["coverage_evidence"]["covered"], true);
    assert!(v["coverage_evidence"]["targets"]
        .as_str()
        .unwrap()
        .contains("message:<id>"));
    assert_eq!(v["coverage_evidence"]["row_count"], 1);
    assert_eq!(
        v["coverage_evidence"]["sample_targets"][0]["target"],
        "message:event-123"
    );
    assert_eq!(
        v["coverage_evidence"]["sample_targets"][0]["also"],
        "event:event-123"
    );
    assert!(v["coverage_evidence"]["columns"]
        .as_array()
        .unwrap()
        .iter()
        .any(|column| column == "message_id"));
    assert_no_check(&v, "seams");
    assert_no_check(&v, "resource_accounting");

    let sample_target = v["coverage_evidence"]["sample_targets"][0]["target"]
        .as_str()
        .unwrap();
    let sampled = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": sample_target }),
    )
    .unwrap();
    assert_check_status(&sampled, "message", "passed");

    let also_target = v["coverage_evidence"]["sample_targets"][0]["also"]
        .as_str()
        .unwrap();
    let also = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": also_target }),
    )
    .unwrap();
    assert_check_status(&also, "event", "passed");
}

#[tokio::test]
async fn rpc_probe_validate_table_samples_delivery_and_publish_alternates() {
    let state = DaemonState::new_for_test().await;
    let failed_outbox_id = state
        .with_store(|s| {
            s.record_message(&RecordMessage {
                message_id: "event-recipient".into(),
                thread_id: "room".into(),
                channel_h: "room".into(),
                author_pubkey: "pk-author".into(),
                body: "hello".into(),
                created_at: 100,
                direction: "inbound".into(),
                sync_state: "accepted".into(),
                native_event_id: Some("event-recipient".into()),
                error: None,
            })?;
            s.add_message_recipient("event-recipient", "pk-recipient", None)?;
            let outbox_id = s.enqueue_outbox(r#"{"id":"event-outbox","kind":9}"#, 101)?;
            s.apply_outbox_projection(outbox_id, "published", None, false)?;
            let failed_outbox_id =
                s.enqueue_outbox(r#"{"id":"event-failed-outbox","kind":9}"#, 102)?;
            s.apply_outbox_projection(
                failed_outbox_id,
                "failed",
                Some("relay rejected event"),
                true,
            )?;
            Ok::<_, anyhow::Error>(failed_outbox_id)
        })
        .unwrap();

    let recipients = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "table:message_recipients" }),
    )
    .unwrap();
    assert_eq!(
        recipients["coverage_evidence"]["sample_targets"][0]["target"],
        "recipient:event-recipient:pk-recipient"
    );
    assert_eq!(
        recipients["coverage_evidence"]["sample_targets"][0]["also"],
        "message:event-recipient"
    );

    let outbox = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "table:outbox" }),
    )
    .unwrap();
    assert_eq!(
        outbox["coverage_evidence"]["sample_targets"][0]["target"],
        format!("outbox:{failed_outbox_id}")
    );
    assert_eq!(
        outbox["coverage_evidence"]["sample_targets"][0]["also"],
        "event:event-failed-outbox"
    );
}

#[tokio::test]
async fn rpc_probe_validate_table_reports_missing_table_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "ledger:no_such_table" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_eq!(v["verdict"], "passed_with_limitations");
    assert_check_status(&v, "table_coverage", "not_proven");
    assert_eq!(v["coverage_evidence"]["present"], false);
    assert_eq!(v["coverage_evidence"]["covered"], false);
    assert!(v["limitations"].as_array().unwrap().iter().any(|row| {
        row.as_str()
            .unwrap()
            .contains("no sqlite application table matched this name")
    }));
}

#[tokio::test]
async fn rpc_probe_validate_table_rejects_empty_table_name() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "table:" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "target", "failed");
    assert_eq!(v["target_evidence"]["kind"], "empty_handle");
    assert!(v["target_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("durable table name"));
}

fn assert_has_table_target(v: &serde_json::Value, table: &str, target_fragment: &str, mode: &str) {
    let row = v["coverage_evidence"]["durable_tables"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["table"] == table)
        .expect("durable table coverage row");
    assert_eq!(row["mode"], mode);
    assert!(row["targets"].as_str().unwrap().contains(target_fragment));
    assert_eq!(row["present"], true);
}

fn assert_has_surface_mode(v: &serde_json::Value, surface: &str, mode: &str) {
    let row = v["coverage_evidence"]["surfaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["surface"] == surface)
        .expect("surface coverage row");
    assert_eq!(row["mode"], mode);
}

fn assert_check_status(v: &serde_json::Value, name: &str, status: &str) {
    assert_eq!(check_row(v, name)["status"], status);
}

fn assert_no_check(v: &serde_json::Value, name: &str) {
    assert!(
        v["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["name"] != name),
        "unexpected check row `{name}` in {:#}",
        v["checks"]
    );
}

fn check_row<'a>(v: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    v["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == name)
        .expect("check row")
}
