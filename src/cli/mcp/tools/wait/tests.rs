use super::*;

#[test]
fn timeouts_must_be_positive_integers() {
    assert_eq!(
        required_timeout(&json!({ "seconds": 3 }), "seconds").unwrap(),
        3
    );
    assert!(required_timeout(&json!({ "seconds": 0 }), "seconds").is_err());
    assert!(required_timeout(&json!({ "seconds": "3" }), "seconds").is_err());
}

#[test]
fn send_timeout_is_optional_but_validated_when_present() {
    assert_eq!(send_timeout(&json!({})).unwrap(), None);
    assert_eq!(
        send_timeout(&json!({ "wait_seconds": 9 })).unwrap(),
        Some(9)
    );
    assert!(send_timeout(&json!({ "wait_seconds": 0 })).is_err());
}

#[test]
fn string_arrays_default_to_empty() {
    assert_eq!(string_array(&json!({}), "channels"), Vec::<Value>::new());
    assert_eq!(
        string_array(&json!({ "channels": ["one", "two"] }), "channels"),
        vec![json!("one"), json!("two")]
    );
}

#[test]
fn ambient_params_preserve_filters_and_session() {
    let params = ambient_params(&json!({
        "timeout_seconds": 60,
        "channels": ["#mosaico/work"],
        "from": "agent",
        "session": "actor",
    }))
    .unwrap();
    assert_eq!(params["timeout_secs"], 60);
    assert_eq!(params["channels"], json!(["#mosaico/work"]));
    assert_eq!(params["from"], "agent");
    assert_eq!(params["session"], "actor");
}

#[test]
fn reply_params_correlate_to_send_and_mentioned_recipients() {
    let send = json!({
        "event_id": "sent-event",
        "mentioned_pubkeys": ["pubkey"],
        "mentioned_labels": ["agent"],
    });
    let params = reply_params(
        &send,
        send["event_id"].as_str().unwrap(),
        120,
        &json!({ "session": "actor" }),
    );
    assert_eq!(params["timeout_secs"], 120);
    assert_eq!(params["reply_to"], "sent-event");
    assert_eq!(params["from_pubkeys"], json!(["pubkey"]));
    assert_eq!(params["from_labels"], json!(["agent"]));
    assert_eq!(params["session"], "actor");
}
