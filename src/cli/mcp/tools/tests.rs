use super::*;

#[test]
fn channel_send_rejects_arguments_outside_its_schema() {
    let error = channel_send_params(&json!({
        "message": "hello",
        "long_message": true,
    }))
    .unwrap_err()
    .to_string();
    assert!(error.contains("unsupported mosaico.channel_send argument"));
    assert!(error.contains("long_message"));
}

#[test]
fn channel_send_projects_correlated_wait_intent_to_the_daemon() {
    let waiting = channel_send_params(&json!({
        "message": "hello",
        "wait_seconds": 30,
    }))
    .unwrap();
    let fire_and_forget = channel_send_params(&json!({
        "message": "hello",
    }))
    .unwrap();

    assert_eq!(waiting["wait_intent"], true);
    assert_eq!(fire_and_forget["wait_intent"], false);
}

#[test]
fn explicit_session_remains_an_operator_override() {
    let params = caller_params(json!({ "session": "explicit" }), Some("remote-actor"));
    assert_eq!(params["session"], "explicit");
}

#[test]
fn remote_actor_is_the_default_session() {
    let params = caller_params(json!({}), Some("remote-actor"));
    assert_eq!(params["session"], "remote-actor");
}
