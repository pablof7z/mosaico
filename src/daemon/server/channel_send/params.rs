use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

const CALLER_FIELDS: &[&str] = &[
    "session",
    "pubkey",
    "pty_session",
    "harness_session",
    "watch_pid",
    "harness",
    "agent",
    "cwd",
];
const SEND_FIELDS: &[&str] = &[
    "message",
    "attachments",
    "tags",
    "force",
    "channel",
    "wait_intent",
];
const REPLY_FIELDS: &[&str] = &["id", "message", "attachments"];

pub(super) fn validate_send(params: &Value) -> Result<()> {
    validate(params, "channel_send", SEND_FIELDS)
}

pub(super) fn validate_reply(params: &Value) -> Result<()> {
    validate(params, "channel_reply", REPLY_FIELDS)
}

pub(in crate::daemon::server) fn caller_params(params: &Value) -> Value {
    let mut caller = Map::new();
    if let Some(source) = params.as_object() {
        for field in CALLER_FIELDS {
            if let Some(value) = source.get(*field) {
                caller.insert((*field).to_string(), value.clone());
            }
        }
    }
    Value::Object(caller)
}

fn validate(params: &Value, method: &str, operation_fields: &[&str]) -> Result<()> {
    let object = params
        .as_object()
        .with_context(|| format!("{method} params must be an object"))?;
    if let Some(field) = object.keys().find(|field| {
        !CALLER_FIELDS.contains(&field.as_str()) && !operation_fields.contains(&field.as_str())
    }) {
        bail!("{method} received unknown field {field:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_and_reply_accept_every_caller_anchor_field() {
        let caller = serde_json::json!({
            "session": "session",
            "pubkey": "pubkey",
            "pty_session": "pty",
            "harness_session": "native",
            "watch_pid": 42,
            "harness": "codex",
            "agent": "worker",
            "cwd": "/work",
        });
        let mut send = caller.clone();
        send.as_object_mut().unwrap().extend(
            serde_json::json!({
                "message": "hello",
                "attachments": [],
                "tags": [],
                "force": false,
                "channel": "#work",
                "wait_intent": true,
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        let mut reply = caller;
        reply.as_object_mut().unwrap().extend(
            serde_json::json!({"id": "abc123", "message": "hello", "attachments": []})
                .as_object()
                .unwrap()
                .clone(),
        );

        validate_send(&send).unwrap();
        validate_reply(&reply).unwrap();
    }

    #[test]
    fn caller_projection_drops_operation_fields() {
        let projected = caller_params(&serde_json::json!({
            "session": "session",
            "message": "chat",
            "unexpected": true,
        }));
        assert_eq!(projected, serde_json::json!({"session": "session"}));
    }
}
