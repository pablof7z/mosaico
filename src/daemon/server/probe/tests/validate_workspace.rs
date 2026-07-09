use super::*;

#[tokio::test]
async fn rpc_probe_validate_workspace_passes_existing_directory() {
    let state = DaemonState::new_for_test().await;
    let dir = tempfile::tempdir().unwrap();
    seed_workspace(&state, "proj", "", dir.path().to_str().unwrap());

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "workspace:proj" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "workspace", "passed");
    assert_eq!(v["workspace_evidence"]["root_channel"], "proj");
    assert_eq!(v["workspace_evidence"]["binding_channel_h"], "proj");
    assert_eq!(v["workspace_evidence"]["path_is_dir"], true);
}

#[tokio::test]
async fn rpc_probe_validate_workspace_inherits_from_parent() {
    let state = DaemonState::new_for_test().await;
    let dir = tempfile::tempdir().unwrap();
    seed_workspace(&state, "proj", "", dir.path().to_str().unwrap());
    state
        .with_store(|s| {
            s.upsert_channel("task", "Task", "", "proj", 101)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "work_root:task" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "workspace", "passed");
    assert_eq!(v["workspace_evidence"]["root_channel"], "proj");
    assert_eq!(v["workspace_evidence"]["binding_channel_h"], "proj");
    assert_eq!(v["workspace_evidence"]["inherited_binding"], true);
}

#[tokio::test]
async fn rpc_probe_validate_workspace_missing_binding_is_not_proven() {
    let state = DaemonState::new_for_test().await;
    state
        .with_store(|s| {
            s.upsert_channel("proj", "proj", "", "", 100)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "workspace:proj" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "workspace", "not_proven");
    assert_eq!(v["workspace_evidence"]["found"], false);
}

#[tokio::test]
async fn rpc_probe_validate_workspace_fails_missing_path() {
    let state = DaemonState::new_for_test().await;
    seed_workspace(&state, "proj", "", "/definitely/not/here/tenex-edge-test");

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "workspace:proj" }),
    )
    .unwrap();

    assert_eq!(v["ok"], false);
    assert_check_status(&v, "workspace", "failed");
    assert_eq!(v["workspace_evidence"]["path_exists"], false);
}

#[tokio::test]
async fn rpc_probe_validate_workspace_reports_local_only_binding_with_limitation() {
    let state = DaemonState::new_for_test().await;
    let dir = tempfile::tempdir().unwrap();
    state
        .with_store(|s| {
            s.upsert_workspace("local-only", dir.path().to_str().unwrap(), 100)?;
            Ok::<(), anyhow::Error>(())
        })
        .unwrap();

    let v = rpc_probe(
        &state,
        &json!({ "verb": "validate", "target": "workspace:local-only" }),
    )
    .unwrap();

    assert_eq!(v["ok"], true);
    assert_check_status(&v, "workspace", "passed");
    assert_eq!(v["workspace_evidence"]["channel_found"], false);
    assert!(v["limitations"].as_array().unwrap().iter().any(|l| {
        l.as_str()
            .unwrap()
            .contains("relay channel metadata is not materialized")
    }));
}

fn seed_workspace(state: &std::sync::Arc<DaemonState>, channel_h: &str, parent: &str, path: &str) {
    state
        .with_store(|s| {
            s.upsert_channel(channel_h, channel_h, "", parent, 100)?;
            s.upsert_workspace(channel_h, path, 100)?;
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
