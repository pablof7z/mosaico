use super::*;

#[test]
fn channel_send_rejects_arguments_outside_its_schema() {
    let error = channel_send_params(
        &json!({
            "message": "hello",
            "long_message": true,
        }),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsupported mosaico.channel_send argument"));
    assert!(error.contains("long_message"));
}

#[test]
fn channel_send_projects_correlated_wait_intent_to_the_daemon() {
    let waiting = channel_send_params(
        &json!({
            "message": "hello",
            "wait_seconds": 30,
        }),
        false,
    )
    .unwrap();
    let fire_and_forget = channel_send_params(
        &json!({
            "message": "hello",
        }),
        false,
    )
    .unwrap();

    assert_eq!(waiting["wait_intent"], true);
    assert_eq!(fire_and_forget["wait_intent"], false);
}

#[test]
fn attachment_specs_are_typed_and_canonicalized() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("proof.txt");
    std::fs::write(&file, "proof").unwrap();
    let attachments = attachment_specs(
        &json!({
            "attachments": [format!("evidence={}", file.display())],
        }),
        true,
    )
    .unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].label, "evidence");
    assert_eq!(attachments[0].path, std::fs::canonicalize(file).unwrap());
}

#[test]
fn remote_mcp_cannot_read_local_attachment_paths() {
    let error = channel_send_params(
        &json!({ "message": "hello", "attachments": ["secret.txt"] }),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsupported mosaico.channel_send argument"));
}

#[test]
fn explicit_session_remains_an_operator_override() {
    let params = caller_params(
        json!({ "session": "explicit" }),
        &json!({ "session": "remote-actor" }),
    );
    assert_eq!(params["session"], "explicit");
}

#[test]
fn remote_actor_is_the_default_session() {
    let params = caller_params(json!({}), &json!({ "session": "remote-actor" }));
    assert_eq!(params["session"], "remote-actor");
}
