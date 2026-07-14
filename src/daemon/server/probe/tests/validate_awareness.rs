use super::*;
use crate::state::RegisterSession;

#[tokio::test]
async fn rpc_probe_validate_awareness_reports_confirmed_channel_roster() {
    let state = DaemonState::new_for_test().await;
    let now = crate::util::now_secs();
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", now)?;
            s.replace_channel_admins("room", &["pk-admin".to_string()], now)?;
            s.replace_channel_members("room", &["pk-agent".to_string()], now)?;
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: "pk-agent".into(),
                agent_slug: "coder".into(),
                channel_h: "room".into(),
                child_pid: Some(42),
                transcript_path: None,
                now,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "awareness:room" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["surface"].is_null());
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "awareness", "passed");
    assert_eq!(v["awareness_evidence"]["channel_confirmed"], true);
    assert_eq!(v["awareness_evidence"]["channel_h"], "room");
    assert_eq!(v["awareness_evidence"]["channel_name"], "Room");
    assert_eq!(v["awareness_evidence"]["row_count"], 1);
    assert_eq!(v["awareness_evidence"]["local_row_count"], 1);
    assert_eq!(v["awareness_evidence"]["member_count"], 2);
}

#[tokio::test]
async fn rpc_probe_validate_awareness_reports_missing_channel_as_not_proven() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "who/missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "awareness", "not_proven");
    assert_eq!(v["awareness_evidence"]["found"], false);
    assert!(v["awareness_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("confirmed relay channel metadata"));
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
