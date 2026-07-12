use super::protocol::required_string;
use anyhow::{Context, Result};
use serde_json::{json, Value};

pub(super) fn list() -> Value {
    json!({ "tools": super::catalog::list() })
}

pub(super) async fn call(params: &Value) -> Result<Value> {
    let name = required_string(params, "name")?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match name.as_str() {
        "tenex_edge.who" => who(&args).await,
        "tenex_edge.channel_list" => channel_list(&args).await,
        "tenex_edge.channel_read" => channel_read(&args).await,
        "tenex_edge.channel_send" => channel_send(&args).await,
        "tenex_edge.react" => react(&args).await,
        "tenex_edge.channel_create" => channel_create(&args).await,
        "tenex_edge.channel_join" => channel_mutation("channel_join", &args).await,
        "tenex_edge.channel_leave" => channel_mutation("channel_leave", &args).await,
        "tenex_edge.channel_switch" => channel_mutation("channel_switch", &args).await,
        other => anyhow::bail!("unknown tool: {other}"),
    };
    Ok(match result {
        Ok(value) => tool_ok(value),
        Err(err) => tool_error(format!("{err:#}")),
    })
}

async fn who(args: &Value) -> Result<Value> {
    let channel = opt_string(args, "channel");
    let all_workspaces = args
        .get("all_workspaces")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let extra = json!({
        "channel": channel,
        "all_workspaces": all_workspaces,
    });
    let params = if channel.is_some() || all_workspaces {
        operator_params(extra)
    } else {
        crate::cli::rpc_params(extra)
    };
    daemon_raw("who", params).await
}

async fn channel_list(args: &Value) -> Result<Value> {
    let channel = match opt_string(args, "channel") {
        Some(channel) => channel,
        None => crate::workspace::resolve_or_bail(&std::env::current_dir().unwrap_or_default())?,
    };
    daemon_raw("channel_list", json!({ "channel": channel })).await
}

async fn channel_read(args: &Value) -> Result<Value> {
    let params = crate::cli::rpc_params(json!({
        "id": opt_string(args, "id"),
        "channel": opt_string(args, "channel"),
        "session": opt_string(args, "session"),
        "since": since_arg(args),
        "limit": args.get("limit").and_then(Value::as_u64).unwrap_or(20),
        "offset": args.get("offset").and_then(Value::as_u64).unwrap_or(0),
        "tail": true,
        "live": false,
    }));
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let mut messages = Vec::new();
    client
        .stream("channel_read", params, |item| messages.push(item))
        .await?;
    Ok(json!({ "messages": messages }))
}

async fn channel_send(args: &Value) -> Result<Value> {
    daemon_identity("channel_send", channel_send_params(args)?).await
}

async fn react(args: &Value) -> Result<Value> {
    let params = with_session(
        json!({
            "id": required_string(args, "message_id")?,
            "emoji": required_string(args, "emoji")?,
        }),
        args,
    );
    daemon_identity("channel_react", params).await
}

async fn channel_create(args: &Value) -> Result<Value> {
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
    )
    .await
}

async fn channel_mutation(method: &str, args: &Value) -> Result<Value> {
    daemon_identity(
        method,
        with_session(
            json!({ "channel": required_string(args, "channel")? }),
            args,
        ),
    )
    .await
}

fn channel_send_params(args: &Value) -> Result<Value> {
    Ok(with_session(
        json!({
            "message": required_string(args, "message")?,
            "tags": args.get("tags").and_then(Value::as_array).cloned().unwrap_or_default(),
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

fn operator_params(extra: Value) -> Value {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    merge(json!({ "cwd": cwd }), extra)
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Some(extra)) = (base.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            base.insert(key.clone(), value.clone());
        }
    }
    base
}

async fn daemon_identity(method: &str, extra: Value) -> Result<Value> {
    daemon_raw(method, crate::cli::rpc_params(extra)).await
}

async fn daemon_raw(method: &str, params: Value) -> Result<Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}
