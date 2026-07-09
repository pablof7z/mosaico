use super::*;
use crate::state::{Identity, RegisterSession};

#[tokio::test]
async fn rpc_probe_validate_profile_reports_profile_identity_and_membership() {
    let state = DaemonState::new_for_test().await;
    seed_identity(&state, "pk-agent", true, true);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "profile:pk-agent" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "identity", "passed");
    assert_eq!(v["identity_evidence"]["profile_found"], true);
    assert_eq!(v["identity_evidence"]["identity_found"], true);
    assert_eq!(v["identity_evidence"]["bound_session_alive"], true);
    assert_eq!(v["identity_evidence"]["member_channel_count"], 1);
}

#[tokio::test]
async fn rpc_probe_validate_agent_slug_resolves_on_host() {
    let state = DaemonState::new_for_test().await;
    seed_identity(&state, "pk-agent", true, true);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "agent:coder" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "identity", "passed");
    assert_eq!(v["identity_evidence"]["resolved_pubkey"], "pk-agent");
    assert_eq!(v["identity_evidence"]["profile_slug"], "coder");
}

#[tokio::test]
async fn rpc_probe_validate_identity_fails_alive_identity_without_live_session() {
    let state = DaemonState::new_for_test().await;
    seed_identity(&state, "pk-agent", true, false);

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "identity:pk-agent" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "identity", "failed");
    assert_eq!(v["identity_evidence"]["inconsistent_alive_identity"], true);
}

fn seed_identity(state: &std::sync::Arc<DaemonState>, pubkey: &str, alive: bool, session: bool) {
    let host = state.host.clone();
    state
        .with_store(|s| {
            s.upsert_profile(pubkey, "coder", "coder", &host, false, 100)?;
            s.upsert_channel("room", "Room", "work room", "", 100)?;
            s.replace_channel_admins("room", &[], 100)?;
            s.replace_channel_members("room", &[pubkey.to_string()], 100)?;
            if session {
                s.upsert_session_row(
                    "s1",
                    &RegisterSession {
                        harness: "codex".into(),
                        external_id_kind: "native".into(),
                        external_id: "native-s1".into(),
                        agent_pubkey: pubkey.into(),
                        agent_slug: "coder".into(),
                        channel_h: "room".into(),
                        child_pid: None,
                        transcript_path: None,
                        resume_id: String::new(),
                        now: 100,
                    },
                )?;
            }
            s.upsert_identity(&Identity {
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                codename: "willow-echo-042".into(),
                session_id: "s1".into(),
                channel_h: "room".into(),
                native_id: "native-s1".into(),
                alive,
                created_at: 100,
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
