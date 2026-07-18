use super::protocol::required_string;
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub(super) fn list() -> Value {
    json!({ "tools": super::catalog::list() })
}

pub(super) async fn call(params: &Value) -> Result<Value> {
    call_as(params, None).await
}

pub(super) async fn call_as(params: &Value, caller: Option<&str>) -> Result<Value> {
    let name = required_string(params, "name")?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name.as_str() {
        "mosaico.my_session" => my_session(caller).await,
        "mosaico.channel_list" => channel_list(&args, caller).await,
        "mosaico.channel_read" => channel_read(&args, caller).await,
        "mosaico.channel_send" => channel_send(&args, caller).await,
        "mosaico.react" => react(&args, caller).await,
        "mosaico.channel_create" => channel_create(&args, caller).await,
        "mosaico.channel_join" => channel_mutation("channel_join", &args, caller).await,
        "mosaico.channel_leave" => channel_mutation("channel_leave", &args, caller).await,
        "mosaico.channel_switch" => channel_mutation("channel_switch", &args, caller).await,
        other => anyhow::bail!("unknown tool: {other}"),
    };
    Ok(match result {
        Ok(value) => tool_ok(value),
        Err(err) => tool_error(format!("{err:#}")),
    })
}

async fn my_session(caller: Option<&str>) -> Result<Value> {
    daemon_identity("my_session", json!({}), caller).await
}

async fn channel_list(args: &Value, caller: Option<&str>) -> Result<Value> {
    let channel = match opt_string(args, "channel") {
        Some(channel) => channel,
        None => crate::daemon::workspace_path::channel_for_path_or_bail(
            &std::env::current_dir().unwrap_or_default(),
        )?,
    };
    daemon_identity("channel_list", json!({ "channel": channel }), caller).await
}

async fn channel_read(args: &Value, caller: Option<&str>) -> Result<Value> {
    let params = caller_params(
        json!({
            "id": opt_string(args, "id"),
            "channel": opt_string(args, "channel"),
            "session": opt_string(args, "session"),
            "since": since_arg(args),
            "limit": args.get("limit").and_then(Value::as_u64).unwrap_or(20),
            "offset": args.get("offset").and_then(Value::as_u64).unwrap_or(0),
            "tail": true,
            "live": false,
        }),
        caller,
    );
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let mut messages = Vec::new();
    client
        .stream("channel_read", params, |item| messages.push(item))
        .await?;
    Ok(json!({ "messages": messages }))
}

async fn channel_send(args: &Value, caller: Option<&str>) -> Result<Value> {
    daemon_identity("channel_send", channel_send_params(args)?, caller).await
}

async fn react(args: &Value, caller: Option<&str>) -> Result<Value> {
    let params = with_session(
        json!({
            "id": required_string(args, "message_id")?,
            "emoji": required_string(args, "emoji")?,
        }),
        args,
    );
    daemon_identity("channel_react", params, caller).await
}

async fn channel_create(args: &Value, caller: Option<&str>) -> Result<Value> {
    let name = required_string(args, "name")?;
    let about = required_string(args, "about")?;
    let agents = agent_specs(args)?;
    daemon_identity(
        "channel_create",
        with_session(
            json!({
                "name": name,
                "about": about,
                "parent_channel": opt_string(args, "parent_channel"),
                "agents": agents,
            }),
            args,
        ),
        caller,
    )
    .await
}

async fn channel_mutation(method: &str, args: &Value, caller: Option<&str>) -> Result<Value> {
    daemon_identity(
        method,
        with_session(
            json!({ "channel": required_string(args, "channel")? }),
            args,
        ),
        caller,
    )
    .await
}

fn channel_send_params(args: &Value) -> Result<Value> {
    Ok(with_session(
        json!({
            "message": required_string(args, "message")?,
            "tags": args.get("tags").and_then(Value::as_array).cloned().unwrap_or_default(),
            "force": args.get("force").and_then(Value::as_bool).unwrap_or(false),
            "channel": opt_string(args, "channel"),
            "long_message": args
                .get("long_message")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        args,
    ))
}

fn with_session(mut value: Value, args: &Value) -> Value {
    if let (Some(obj), Some(session)) = (value.as_object_mut(), opt_string(args, "session")) {
        obj.insert("session".into(), json!(session));
    }
    value
}

fn agent_specs(args: &Value) -> Result<Vec<Value>> {
    args.get("agents")
        .and_then(Value::as_array)
        .map(|agents| {
            agents
                .iter()
                .map(|agent| {
                    let raw = agent
                        .as_str()
                        .context("agents entries must be strings like slug@backend")?;
                    let parsed = crate::idref::parse_agent_backend_ref(raw)
                        .with_context(|| format!("malformed agent {raw:?}"))?;
                    let backend = parsed
                        .backend
                        .with_context(|| format!("agent {raw:?} must include @backend"))?;
                    Ok(json!({ "slug": parsed.slug, "backend": backend }))
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn tool_ok(value: Value) -> Value {
    let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": object_content(value),
        "isError": false,
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

fn object_content(value: Value) -> Value {
    if value.is_object() {
        value
    } else {
        json!({ "value": value })
    }
}

fn opt_string(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
}

fn since_arg(args: &Value) -> Option<u64> {
    args.get("since").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().map(super::super::admin::parse_since))
    })
}

async fn daemon_identity(method: &str, extra: Value, caller: Option<&str>) -> Result<Value> {
    daemon_raw(method, caller_params(extra, caller)).await
}

fn caller_params(mut extra: Value, caller: Option<&str>) -> Value {
    if let (Some(caller), Some(object)) = (caller, extra.as_object_mut()) {
        object.entry("session").or_insert_with(|| json!(caller));
    }
    crate::cli::rpc_params(extra)
}

async fn daemon_raw(method: &str, params: Value) -> Result<Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
