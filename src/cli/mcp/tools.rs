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

pub(in crate::cli) async fn call_for_pi(params: &Value, identity: Value) -> Result<Value> {
    call_with_policy(params, identity, true).await
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
    let result = match name.as_str() {
        "mosaico.my_session" => my_session(&identity).await,
        "mosaico.wait" => wait::ambient(&args, &identity).await,
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
        Ok(value) => tool_ok(value),
        Err(err) => tool_error(format!("{err:#}")),
    })
}

async fn my_session(identity: &Value) -> Result<Value> {
    daemon_identity("my_session", json!({}), identity).await
}

async fn channel_list(args: &Value, identity: &Value) -> Result<Value> {
    daemon_identity(
        "channel_list",
        with_session(
            json!({
                "workspace": opt_string(args, "workspace"),
                "all": args.get("all").and_then(Value::as_bool).unwrap_or(false),
                "recursive": args
                    .get("recursive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            }),
            args,
        ),
        identity,
    )
    .await
}

async fn channel_read(args: &Value, identity: &Value) -> Result<Value> {
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
        identity,
    );
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    let mut messages = Vec::new();
    client
        .stream("channel_read", params, |item| messages.push(item))
        .await?;
    Ok(json!({ "messages": messages }))
}

async fn dispatch(args: &Value, identity: &Value) -> Result<Value> {
    let params = with_session(
        json!({
            "target": required_string(args, "target")?,
            "workspace": required_string(args, "workspace")?,
            "channels": args
                .get("channels")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            "message": required_string(args, "message")?,
        }),
        args,
    );
    daemon_identity("dispatch", params, identity).await
}

async fn react(args: &Value, identity: &Value) -> Result<Value> {
    let params = with_session(
        json!({
            "id": required_string(args, "message_id")?,
            "emoji": required_string(args, "emoji")?,
        }),
        args,
    );
    daemon_identity("channel_react", params, identity).await
}

async fn channel_create(args: &Value, identity: &Value) -> Result<Value> {
    daemon_identity(
        "channel_create",
        with_session(
            json!({
                "channel": required_string(args, "channel")?,
                "about": required_string(args, "about")?,
                "agents": agent_specs(args)?,
            }),
            args,
        ),
        identity,
    )
    .await
}

async fn channel_mutation(method: &str, args: &Value, identity: &Value) -> Result<Value> {
    daemon_identity(
        method,
        with_session(
            json!({ "channel": required_string(args, "channel")? }),
            args,
        ),
        identity,
    )
    .await
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
