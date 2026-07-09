use super::*;
use crate::state::NewChannelReadinessAttempt;

#[tokio::test]
async fn rpc_probe_validate_channel_reports_relay_cache_evidence() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.replace_channel_admins("room", &["pk-admin".to_string()], 101)?;
            s.replace_channel_members("room", &["pk-member".to_string()], 102)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "channel:room" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["surface"].is_null());
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "channel", "passed");
    assert_eq!(v["channel_evidence"]["found"], true);
    assert_eq!(v["channel_evidence"]["channel_h"], "room");
    assert_eq!(v["channel_evidence"]["human_name"], "Room");
    assert_eq!(v["channel_evidence"]["membership_snapshot"], true);
    assert_eq!(v["channel_evidence"]["admin_count"], 1);
    assert_eq!(v["channel_evidence"]["member_count"], 2);
    assert_eq!(v["channel_evidence"]["reason"], "");
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|l| !l.as_str().unwrap().contains("host/provider side effect")));
}

#[tokio::test]
async fn rpc_probe_validate_channel_surfaces_provider_attempt_trace() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.replace_channel_admins("room", &["pk-admin".to_string()], 101)?;
            s.replace_channel_members("room", &["pk-member".to_string()], 102)?;
            s.record_channel_readiness_attempt(&NewChannelReadinessAttempt {
                channel_h: "room".into(),
                expect_member: "pk-member".into(),
                parent_hint: None,
                name: Some("Room".into()),
                source: "provider.ensure_channel_ready".into(),
                outcome: "ready".into(),
                reason: "channel readiness verified".into(),
                created_at: 103,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "channel:room" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "channel", "passed");
    assert_eq!(v["channel_evidence"]["provider_attempt_count"], 1);
    assert_eq!(
        v["channel_evidence"]["provider_attempt_rows"][0]["outcome"],
        "ready"
    );
    assert!(v["channel_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("provider_attempt:<id>"));
}

#[tokio::test]
async fn rpc_probe_validate_channel_reports_missing_relay_cache_state() {
    let state = DaemonState::new_for_test().await;

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "channel/missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert!(v["target_evidence"].is_null());
    assert_check_status(&v, "channel", "not_proven");
    assert_eq!(v["channel_evidence"]["found"], false);
    assert!(v["channel_evidence"]["summary"]
        .as_str()
        .unwrap()
        .contains("not materialized"));
}

#[tokio::test]
async fn rpc_probe_validate_readiness_passes_for_hydrated_channel() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.replace_channel_admins("room", &["pk-admin".to_string()], 101)?;
            s.replace_channel_members("room", &["pk-member".to_string()], 102)?;
            s.record_channel_readiness_attempt(&NewChannelReadinessAttempt {
                channel_h: "room".into(),
                expect_member: "pk-member".into(),
                parent_hint: None,
                name: None,
                source: "provider.ensure_channel_ready".into(),
                outcome: "degraded".into(),
                reason: "historical relay timeout".into(),
                created_at: 99,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "readiness:room" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "channel_readiness", "passed");
    assert_eq!(v["channel_evidence"]["kind"], "readiness");
    assert_eq!(v["channel_evidence"]["readiness_ok"], true);
    assert_eq!(v["channel_evidence"]["provider_degraded_count"], 1);
    assert!(v["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .all(|l| !l.as_str().unwrap_or("").contains("timeout")));
}

#[tokio::test]
async fn rpc_probe_validate_readiness_reports_session_start_channel_ready_failure() {
    let state = DaemonState::new_for_test().await;
    {
        let mut r = state.session_start.lock().expect("session_start mutex");
        r.drive(InputFact::SessionStartRequested(session_start_request(
            "s1", "missing",
        )))
        .unwrap();
        r.drive(InputFact::SessionStartFailed(
            crate::reconcile::SessionStartFailedFact {
                session_id: "s1".into(),
                stage: "channel_ready".into(),
                error: "relay rejected event: timeout".into(),
                at: 101,
            },
        ))
        .unwrap();
    }

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "channel_ready:missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "channel_readiness", "failed");
    assert_eq!(v["channel_evidence"]["found"], false);
    assert_eq!(v["channel_evidence"]["channel_ready_failure_count"], 1);
    assert_eq!(
        v["channel_evidence"]["session_start_rows"][0]["failure_stage"],
        "channel_ready"
    );
    assert!(v["channel_evidence"]["readiness_reason"]
        .as_str()
        .unwrap()
        .contains("timeout"));
}

#[tokio::test]
async fn rpc_probe_validate_readiness_reports_provider_degraded_attempt() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.record_channel_readiness_attempt(&NewChannelReadinessAttempt {
                channel_h: "missing".into(),
                expect_member: "pk-member".into(),
                parent_hint: Some("root".into()),
                name: Some("Missing".into()),
                source: "provider.ensure_channel_ready".into(),
                outcome: "degraded".into(),
                reason: "management key is not admin and self-grant failed".into(),
                created_at: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "readiness:missing" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "channel_readiness", "failed");
    assert_eq!(v["channel_evidence"]["found"], false);
    assert_eq!(v["channel_evidence"]["provider_attempt_count"], 1);
    assert_eq!(v["channel_evidence"]["provider_degraded_count"], 1);
    assert_eq!(
        v["channel_evidence"]["provider_attempt_rows"][0]["outcome"],
        "degraded"
    );
    assert!(v["channel_evidence"]["readiness_reason"]
        .as_str()
        .unwrap()
        .contains("self-grant failed"));
}

fn session_start_request(
    session_id: &str,
    channel_h: &str,
) -> crate::reconcile::SessionStartRequestFact {
    crate::reconcile::SessionStartRequestFact {
        session_id: session_id.to_string(),
        agent: "coder".into(),
        harness: "codex".into(),
        external_id_kind: "harness_session".into(),
        external_id: format!("native-{session_id}"),
        native_id: format!("native-{session_id}"),
        work_root: "/tmp/work".into(),
        channel_h: channel_h.to_string(),
        channel_for_upsert: channel_h.to_string(),
        rel_cwd: ".".into(),
        room_parent: None,
        channel_provision_name: None,
        watch_pid: Some(42),
        pty_session: None,
        ring_doorbell: false,
        signer_pubkey: "pk-signer".into(),
        signer_label: "coder".into(),
        already_running: false,
        channel_already_subscribed: false,
        at: 100,
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
