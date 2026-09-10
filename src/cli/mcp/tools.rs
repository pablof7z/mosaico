use super::protocol::required_string;
use anyhow::{Context, Result};
use serde_json::{json, Value};

mod search;
mod send;
mod wait;

#[cfg(test)]
use send::{attachment_specs, channel_send_params};

pub(super) fn list() -> Value {
    json!({ "tools": super::catalog::list() })
}

pub(super) async fn call(params: &Value) -> Result<Value> {
    call_with_policy(params, crate::cli::caller_identity(), false).await
}

pub(super) async fn call_as(params: &Value, caller: Option<&str>) -> Result<Value> {
    let mut identity = crate::cli::caller_identity();
    if let (Some(caller), Some(object)) = (caller, identity.as_object_mut()) {
        object.insert("session".into(), json!(caller));
    }
    call_with_policy(params, identity, false).await
}

async fn call_with_policy(
    params: &Value,
    identity: Value,
    allow_local_attachments: bool,
) -> Result<Value> {
    let name = required_string(params, "name")?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if name == "mosaico.skill" {
        return Ok(
            match super::skill::tool_result(opt_string(&args, "name").as_deref()) {
                Ok(value) => value,
                Err(err) => tool_error(format!("{err:#}")),
            },
        );
    }
    if name == "mosaico.channel_search" {
        return Ok(match search::call(&args, &identity).await {
            Ok(value) => value,
            Err(err) => tool_error(format!("{err:#}")),
        });
    }
    let result: Result<String> = match name.as_str() {
        "mosaico.my_session" => my_session(&identity).await,
        "mosaico.wait" => wait_ambient(&args, &identity).await,
        "mosaico.channel_list" => channel_list(&args, &identity).await,
        "mosaico.channel_read" => channel_read(&args, &identity).await,
        "mosaico.channel_send" => {
            send::channel_send(&args, &identity, allow_local_attachments).await
        }
        "mosaico.dispatch" => dispatch(&args, &identity).await,
        "mosaico.react" => react(&args, &identity).await,
        "mosaico.channel_create" => channel_create(&args, &identity).await,
        "mosaico.channel_join" => channel_mutation("channel_join", &args, &identity).await,
        "mosaico.channel_leave" => channel_mutation("channel_leave", &args, &identity).await,
        other => anyhow::bail!("unknown tool: {other}"),
    };
    Ok(match result {
        Ok(text) => tool_ok(text),
        Err(err) => tool_error(format!("{err:#}")),
    })
}

async fn my_session(identity: &Value) -> Result<String> {
    let v = daemon_identity("my_session", json!({}), identity).await?;
    Ok(v["fabric"].as_str().unwrap_or_default().to_string())
}

async fn wait_ambient(args: &Value, identity: &Value) -> Result<String> {
    let v = wait::ambient(args, identity).await?;
    crate::channel_render::format_wait_result(&v, crate::util::now_secs())
}

async fn channel_list(args: &Value, identity: &Value) -> Result<String> {
    let v = daemon_identity(
        "channel_list",
        json!({
            "workspace": opt_string(args, "workspace"),
            "all": args.get("all").and_then(Value::as_bool).unwrap_or(false),
            "recursive": args
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }),
        identity,
    )
    .await?;
    Ok(crate::channel_render::format_channel_list(&v))
}

async fn channel_read(args: &Value, identity: &Value) -> Result<String> {
    let params = caller_params(
        json!({
            "id": opt_string(args, "id"),
            "channel": opt_string(args, "channel"),
            "since": since_arg(args),
            "limit": args.get("limit").and_then(Value::as_u64).unwrap_or(20),
            "offset": args.get("offset").and_then(Value::as_u64).unwrap_or(0),
            "tail": true,
            "live": false,
        }),
        identity,
    );
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let mut rows = Vec::new();
    client
        .stream("channel_read", params, |item| rows.push(item))
        .await?;
    Ok(rows
        .iter()
        .map(|row| crate::channel_render::render_channel_read_row(row, false))
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn dispatch(args: &Value, identity: &Value) -> Result<String> {
    let params = json!({
        "target": required_string(args, "target")?,
        "workspace": required_string(args, "workspace")?,
        "channels": args
            .get("channels")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        "message": required_string(args, "message")?,
    });
    let v = daemon_identity("dispatch", params, identity).await?;
    let dispatch_id = crate::idref::event_short_id(v["dispatch_event_id"].as_str().unwrap_or("?"));
    let message_id = crate::idref::event_short_id(v["message_event_id"].as_str().unwrap_or("?"));
    let workspace = v["workspace"].as_str().unwrap_or("?");
    let route = v["route_channel"].as_str().unwrap_or("?");
    Ok(format!(
        "dispatched {dispatch_id} in {workspace} via {route}; message {message_id}"
    ))
}

async fn react(args: &Value, identity: &Value) -> Result<String> {
    let params = json!({
        "id": required_string(args, "id")?,
        "emoji": required_string(args, "emoji")?,
    });
    let v = daemon_identity("channel_react", params, identity).await?;
    let id = v["id"].as_str().unwrap_or("?");
    let target = v["target"].as_str().unwrap_or("?");
    let emoji = v["emoji"].as_str().unwrap_or("?");
    Ok(format!("reacted {emoji} {id} to {target}"))
}

async fn channel_create(args: &Value, identity: &Value) -> Result<String> {
    let v = daemon_identity(
        "channel_create",
        json!({
            "channel": required_string(args, "channel")?,
            "about": required_string(args, "about")?,
            "agents": agent_specs(args)?,
        }),
        identity,
    )
    .await?;
    let channel = v["channel"].as_str().unwrap_or("?");
    let oid = v["orchestration_event_id"].as_str().unwrap_or("");
    let mut text = if v["joined"].as_bool().unwrap_or(false) {
        format!("{channel} created and joined")
    } else {
        format!("{channel} created")
    };
    if !oid.is_empty() {
        let short = &oid[..oid.len().min(8)];
        text.push_str(&format!("\n  orchestration kind:9 {short}"));
    }
    Ok(text)
}

async fn channel_mutation(method: &str, args: &Value, identity: &Value) -> Result<String> {
    let v = daemon_identity(
        method,
        json!({ "channel": required_string(args, "channel")? }),
        identity,
    )
    .await?;
    if method == "channel_join" {
        let channel = v["channel"].as_str().unwrap_or("?");
        let mut text = format!("joined channel {channel}");
        if let Some(notice) = v["history_notice"].as_str().filter(|n| !n.is_empty()) {
            text.push('\n');
            text.push_str(notice);
        }
        Ok(text)
    } else {
        let channel = v["channel"].as_str().unwrap_or("?");
        Ok(format!("left channel {channel}"))
    }
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

fn tool_ok(text: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false,
    })
}

fn tool_error(message: String) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
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

async fn daemon_identity(method: &str, extra: Value, identity: &Value) -> Result<Value> {
    daemon_raw(method, caller_params(extra, identity)).await
}

fn caller_params(extra: Value, identity: &Value) -> Value {
    crate::cli::context::merge_rpc_params(identity.clone(), extra)
}

async fn daemon_raw(method: &str, params: Value) -> Result<Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

#[cfg(test)]
#[path = "tools/tests.rs"]
mod tests;
