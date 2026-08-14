use super::*;

#[test]
fn classifies_response_and_terminal_event() {
    let response = serde_json::json!({
        "id":"41", "type":"response", "command":"get_state", "success":true,
        "data":{"sessionId":"pi-session"}
    });
    match protocol::classify_for(Dialect::PiRpc, response) {
        Inbound::Response { id, result } => {
            assert_eq!(id, 41);
            assert_eq!(result.unwrap()["sessionId"], "pi-session");
        }
        _ => panic!("expected Pi response"),
    }

    let event = serde_json::json!({"type":"agent_end", "messages":[]});
    match protocol::classify_for(Dialect::PiRpc, event) {
        Inbound::Notification { method, params } => {
            assert_eq!(method, "agent_end");
            assert_eq!(params["type"], "agent_end");
        }
        _ => panic!("expected Pi event"),
    }
}

#[test]
fn preserves_rejection_error() {
    let response = serde_json::json!({
        "id":"7", "type":"response", "command":"prompt", "success":false,
        "error":"agent is already streaming"
    });
    match protocol::classify_for(Dialect::PiRpc, response) {
        Inbound::Response { result, .. } => {
            assert_eq!(result.unwrap_err().message, "agent is already streaming");
        }
        _ => panic!("expected Pi rejection"),
    }
}
