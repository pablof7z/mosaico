use super::*;
use crate::state::receipts::NewReceipt;
use crate::state::RegisterSession;

mod render;

#[tokio::test]
async fn rpc_probe_validate_hook_target_matches_live_graph_to_receipt() {
    let state = DaemonState::new_for_test().await;
    seed_hook_graph_and_receipt(&state, "s1", None);

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "hook_context_outcome", "passed");
    assert_check_status(&v, "why", "passed");
    assert_check_status(&v, "explain", "passed");
    assert_eq!(v["hook_context_evidence"]["graph_found"], true);
    assert_eq!(v["hook_context_evidence"]["receipt_found"], true);
    assert_eq!(v["hook_context_evidence"]["revision_matches_receipt"], true);
}

#[tokio::test]
async fn rpc_probe_validate_hook_target_fails_receipt_revision_mismatch() {
    let state = DaemonState::new_for_test().await;
    seed_hook_graph_and_receipt(&state, "s1", Some(999));

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "hook_context_outcome", "failed");
    assert_eq!(v["hook_context_evidence"]["graph_found"], true);
    assert_eq!(v["hook_context_evidence"]["receipt_found"], true);
    assert_eq!(
        v["hook_context_evidence"]["revision_matches_receipt"],
        false
    );
    assert!(v["hook_context_evidence"]["reason"]
        .as_str()
        .unwrap()
        .contains("does not match"));
}

#[tokio::test]
async fn rpc_probe_validate_hook_target_fails_historical_receipt_without_live_graph() {
    let state = DaemonState::new_for_test().await;
    record_hook_receipt(
        &state,
        "s1",
        1,
        r#"{"kind":"turn_start","frame":"baseline"}"#,
    );

    let v = rpc_probe(&state, &json!({ "verb": "validate", "target": "hook:s1" })).unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "hook_context_outcome", "failed");
    assert_check_status(&v, "explain", "passed");
    assert_eq!(v["hook_context_evidence"]["graph_found"], false);
    assert_eq!(v["hook_context_evidence"]["receipt_found"], true);
}

fn seed_session_row(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    channel_h: &str,
    confirmed_channel: bool,
) {
    state
        .with_store(|s| {
            if confirmed_channel {
                s.upsert_channel(channel_h, channel_h, "", "", 100)?;
                s.replace_channel_admins(channel_h, &["pk-admin".to_string()], 100)?;
                s.replace_channel_members(channel_h, &["pk1".to_string(), "pk2".to_string()], 100)?;
            }
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                channel_h: channel_h.into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_session_channel_without_roster(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    channel_h: &str,
) {
    state
        .with_store(|s| {
            s.upsert_channel(channel_h, channel_h, "", "", 100)?;
            s.reserve_session(&RegisterSession {
                harness: "codex".into(),
                pubkey: pubkey.into(),
                agent_slug: "coder".into(),
                channel_h: channel_h.into(),
                child_pid: None,
                transcript_path: None,
                now: 100,
            })?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();
}

fn seed_hook_graph_and_receipt(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    override_revision: Option<i64>,
) {
    let inputs: ViewInputs = serde_json::from_value(json!({
        "meta": {
            "self_row": null,
            "workspace": { "name": "", "channel": "", "about": "" },
            "agents": [],
            "channels": [],
            "other_workspaces": [],
            "warnings": ["hook validation"],
            "self_pubkey": "",
            "self_ref": "",
            "force": true
        },
        "members": { "roster": {}, "refs": {}, "backend": [] },
        "presence": { "statuses": {} },
        "messages": { "channels": {} }
    }))
    .unwrap();
    seed_hook_graph_and_receipt_with_inputs(state, pubkey, inputs, override_revision);
}

fn seed_hook_graph_and_receipt_with_inputs(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    inputs: ViewInputs,
    override_revision: Option<i64>,
) {
    let outcome = state
        .hook_contexts
        .lock()
        .unwrap()
        .entry(pubkey.into())
        .or_default()
        .render_context(pubkey, "turn_start", 0, 100, inputs)
        .unwrap();
    let revision = override_revision.unwrap_or(outcome.revision);
    record_hook_receipt(
        state,
        pubkey,
        revision,
        &outcome.receipt.to_json().to_string(),
    );
}

fn unconfirmed_channel_inputs() -> ViewInputs {
    serde_json::from_value(json!({
        "meta": {
            "self_row": null,
            "workspace": { "name": "ghost", "channel": "ghost", "about": "" },
            "agents": [],
            "channels": [{
                "h": "ghost",
                "name": "ghost",
                "reference": "ghost",
                "about": "",
                "subchannels": []
            }],
            "other_workspaces": [],
            "warnings": [],
            "self_pubkey": "",
            "self_ref": "",
            "force": true
        },
        "members": { "roster": {}, "refs": {}, "backend": [] },
        "presence": { "statuses": {} },
        "messages": { "channels": {} }
    }))
    .unwrap()
}

fn local_agents_and_members_inputs() -> ViewInputs {
    serde_json::from_value(json!({
        "meta": {
            "self_row": {
                "agent": "coder",
                "backend": "laptop",
                "pubkey": "s1"
            },
            "workspace": { "name": "room", "channel": "room", "about": "" },
            "agents": [{
                "reference": "helper",
                "about": "available",
                "created_at": 1
            }],
            "channels": [{
                "h": "room",
                "name": "room",
                "reference": "room",
                "about": "",
                "subchannels": []
            }],
            "other_workspaces": [],
            "warnings": [],
            "self_pubkey": "pk1",
            "self_ref": "coder",
            "force": true
        },
        "members": {
            "roster": { "room": { "pk1": "member", "pk2": "member" } },
            "refs": { "pk1": "coder", "pk2": "reviewer" },
            "backend": []
        },
        "presence": { "statuses": {} },
        "messages": { "channels": {} }
    }))
    .unwrap()
}

fn record_hook_receipt(
    state: &std::sync::Arc<DaemonState>,
    pubkey: &str,
    revision: i64,
    changed_summary: &str,
) {
    state
        .with_store(|s| {
            s.record_receipt(&NewReceipt {
                surface: "hook_context".into(),
                transaction_id: 7,
                revision,
                changed_summary: changed_summary.into(),
                commands: "[]".into(),
                artifact_ref: Some(format!("{pubkey}:turn_start:100")),
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
