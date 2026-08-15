//! Native JSON boundary used by the `pi-mosaico` extension.
//!
//! Human CLI rendering never crosses this boundary. One invocation reads one
//! request to EOF and writes one Pi `AgentToolResult`-shaped JSON response.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{Read, Write};

const VERSION: u8 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u8,
    tool: String,
    #[serde(default = "empty_object")]
    arguments: Value,
    session: PiSession,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PiSession {
    native_id: String,
    cwd: String,
    #[serde(default)]
    pty_session: Option<String>,
}

pub(super) async fn run() -> Result<()> {
    let response = match read_request().and_then(normalize) {
        Ok((call, identity)) => match super::mcp::tools::call_for_pi(&call, identity).await {
            Ok(result) => pi_result(result),
            Err(error) => error_result(format!("{error:#}")),
        },
        Err(error) => error_result(format!("{error:#}")),
    };
    serde_json::to_writer(std::io::stdout().lock(), &response)?;
    std::io::stdout().lock().write_all(b"\n")?;
    Ok(())
}

fn read_request() -> Result<Request> {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    serde_json::from_str(&input).context("parsing Pi harness request JSON")
}

fn normalize(request: Request) -> Result<(Value, Value)> {
    if request.version != VERSION {
        anyhow::bail!(
            "unsupported Pi harness protocol version {}; expected {VERSION}",
            request.version
        );
    }
    if request.session.native_id.trim().is_empty() {
        anyhow::bail!("Pi native session id must not be empty");
    }
    if request.session.cwd.trim().is_empty() {
        anyhow::bail!("Pi session cwd must not be empty");
    }
    let (name, arguments) = operation(&request.tool, request.arguments)?;
    let identity = json!({
        "session": Value::Null,
        "pty_session": request.session.pty_session,
        "harness": "pi",
        "harness_session": request.session.native_id,
        "agent": super::agent_env_slug(),
        "cwd": request.session.cwd,
    });
    Ok((json!({ "name": name, "arguments": arguments }), identity))
}

fn operation(tool: &str, mut arguments: Value) -> Result<(&'static str, Value)> {
    let (name, allowed): (&str, &[&str]) = match tool {
        "mosaico_session" => ("mosaico.my_session", &[]),
        "mosaico_wait" => ("mosaico.wait", &["timeout_seconds", "channels", "from"]),
        "mosaico_channel_list" => ("mosaico.channel_list", &["workspace", "all", "recursive"]),
        "mosaico_channel_read" => ("mosaico.channel_read", &["channel", "id", "since", "limit"]),
        "mosaico_channel_search" => (
            "mosaico.channel_search",
            &[
                "from", "to", "contains", "channels", "since", "until", "limit", "cursor",
            ],
        ),
        "mosaico_send" => (
            "mosaico.channel_send",
            &[
                "message",
                "channel",
                "tags",
                "attachments",
                "force",
                "wait_seconds",
            ],
        ),
        "mosaico_reply" => {
            validate_arguments(tool, &arguments, &["message_id", "message", "attachments"])?;
            let object = arguments
                .as_object_mut()
                .context("mosaico_reply arguments must be an object")?;
            let message_id = object
                .remove("message_id")
                .and_then(|value| value.as_str().map(str::to_string))
                .filter(|value| !value.trim().is_empty())
                .context("mosaico_reply requires message_id")?;
            object.insert("reply_to".into(), json!(message_id));
            return Ok(("mosaico.channel_send", arguments));
        }
        "mosaico_react" => ("mosaico.react", &["message_id", "emoji"]),
        "mosaico_channel_create" => ("mosaico.channel_create", &["channel", "about", "agents"]),
        "mosaico_channel_join" => ("mosaico.channel_join", &["channel"]),
        "mosaico_channel_leave" => ("mosaico.channel_leave", &["channel"]),
        "mosaico_dispatch" => (
            "mosaico.dispatch",
            &["target", "workspace", "channels", "message"],
        ),
        other => anyhow::bail!("unsupported Pi agent tool {other:?}"),
    };
    validate_arguments(tool, &arguments, allowed)?;
    if tool == "mosaico_channel_list" {
        let selected = ["workspace", "all", "recursive"]
            .iter()
            .filter(|key| {
                arguments.get(**key).is_some_and(|value| match value {
                    Value::Bool(value) => *value,
                    Value::String(value) => !value.trim().is_empty(),
                    _ => false,
                })
            })
            .count();
        if selected > 1 {
            anyhow::bail!("mosaico_channel_list accepts only one of workspace, all, or recursive");
        }
    }
    Ok((name, arguments))
}

fn validate_arguments(tool: &str, arguments: &Value, allowed: &[&str]) -> Result<()> {
    let object = arguments
        .as_object()
        .with_context(|| format!("{tool} arguments must be an object"))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        anyhow::bail!("unsupported {tool} argument {unknown:?}");
    }
    Ok(())
}

fn pi_result(result: Value) -> Value {
    json!({
        "content": result.get("content").cloned().unwrap_or_else(|| json!([])),
        "details": result.get("structuredContent").cloned().unwrap_or(Value::Null),
        "isError": result.get("isError").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn error_result(message: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "details": { "error": message },
        "isError": true,
    })
}

fn empty_object() -> Value {
    json!({})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(tool: &str, arguments: Value) -> Request {
        Request {
            version: VERSION,
            tool: tool.into(),
            arguments,
            session: PiSession {
                native_id: "native-pi-id".into(),
                cwd: "/workspace".into(),
                pty_session: Some("pty-id".into()),
            },
        }
    }

    #[test]
    fn reply_becomes_the_existing_structured_reply_operation() {
        let (call, identity) = normalize(request(
            "mosaico_reply",
            json!({ "message_id": "abcd", "message": "done" }),
        ))
        .unwrap();
        assert_eq!(call["name"], "mosaico.channel_send");
        assert_eq!(call["arguments"]["reply_to"], "abcd");
        assert_eq!(call["arguments"]["message"], "done");
        assert_eq!(identity["harness"], "pi");
        assert_eq!(identity["harness_session"], "native-pi-id");
        assert_eq!(identity["pty_session"], "pty-id");
    }

    #[test]
    fn operator_tools_are_not_admitted() {
        let error = normalize(request("mosaico_daemon_restart", json!({})))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unsupported Pi agent tool"));
    }

    #[test]
    fn agent_tools_cannot_override_session_identity() {
        let error = normalize(request(
            "mosaico_send",
            json!({ "message": "hello", "session": "someone-else" }),
        ))
        .unwrap_err()
        .to_string();
        assert!(error.contains("unsupported mosaico_send argument \"session\""));
    }

    #[test]
    fn response_preserves_structured_details_and_error_state() {
        let response = pi_result(json!({
            "content": [{ "type": "text", "text": "sent" }],
            "structuredContent": { "event_id": "event" },
            "isError": false,
        }));
        assert_eq!(response["details"]["event_id"], "event");
        assert_eq!(response["isError"], false);
    }
}
