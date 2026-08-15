//! Pi's native agent-only tool surface over the daemon UDS.
//!
//! This is not a CLI adapter: values stay structured from Pi through the
//! daemon, and all channel, attachment, identity, and dispatch policy remains
//! in the Rust owner.

use super::*;
use serde_json::{json, Map, Value};
mod args;
use args::*;

pub(super) async fn rpc_call(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    let result = execute(state, params).await;
    Ok(match result {
        Ok(value) => tool_ok(value),
        Err(error) => tool_error(format!("{error:#}")),
    })
}

async fn execute(state: &Arc<DaemonState>, params: &Value) -> Result<Value> {
    validate_caller(params)?;
    let tool = required(params, "tool")?;
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let caller = caller(params)?;
    match tool {
        "mosaico_session" => {
            allow_only(&args, &[])?;
            super::rpc_my_session(state, &Value::Object(caller))
        }
        "mosaico_wait" => {
            allow_only(&args, &["timeout_seconds", "channels", "from"])?;
            let mut call = caller;
            insert(
                &mut call,
                "timeout_secs",
                required_u64(&args, "timeout_seconds")?,
            );
            insert(&mut call, "channels", array(&args, "channels")?);
            insert(&mut call, "from", optional(&args, "from"));
            super::channel_wait::rpc_channel_wait(state, &Value::Object(call)).await
        }
        "mosaico_channel_list" => {
            allow_only(&args, &["workspace", "all", "recursive"])?;
            let selected = ["workspace", "all", "recursive"]
                .into_iter()
                .filter(|key| enabled(&args, key))
                .count();
            anyhow::ensure!(selected <= 1, "channel list accepts one selector");
            let mut call = caller;
            insert(&mut call, "workspace", optional(&args, "workspace"));
            insert(&mut call, "all", bool_or(&args, "all", false));
            insert(&mut call, "recursive", bool_or(&args, "recursive", false));
            super::rpc_channel_list(state, &Value::Object(call))
        }
        "mosaico_channel_search" => {
            allow_only(
                &args,
                &[
                    "from", "to", "contains", "channels", "since", "until", "limit", "cursor",
                ],
            )?;
            let cursor = optional(&args, "cursor");
            if !cursor.is_null()
                && [
                    "from", "to", "contains", "channels", "since", "until", "limit",
                ]
                .into_iter()
                .any(|key| args.get(key).is_some())
            {
                anyhow::bail!("cursor must be used alone; it already contains the search query");
            }
            let mut call = Map::new();
            for key in ["from", "to", "contains", "channels"] {
                call.insert(key.into(), array(&args, key)?);
            }
            call.insert("since".into(), search_time(&args, "since")?);
            call.insert("until".into(), search_time(&args, "until")?);
            call.insert("limit".into(), optional(&args, "limit"));
            call.insert("cursor".into(), cursor);
            super::channel_search::rpc_channel_search(state, &Value::Object(call))
        }
        "mosaico_send" => send(state, &caller, &args).await,
        "mosaico_reply" => reply(state, &caller, &args).await,
        "mosaico_react" => {
            allow_only(&args, &["message_id", "emoji"])?;
            let mut call = caller;
            insert(&mut call, "id", required(&args, "message_id")?);
            insert(&mut call, "emoji", required(&args, "emoji")?);
            super::channel_send::rpc_channel_react(state, &Value::Object(call)).await
        }
        "mosaico_channel_create" => {
            allow_only(&args, &["channel", "about", "agents"])?;
            let mut call = caller;
            insert(&mut call, "channel", required(&args, "channel")?);
            insert(&mut call, "about", required(&args, "about")?);
            insert(&mut call, "agents", agent_specs(&args)?);
            super::rpc_channel_create(state, &Value::Object(call)).await
        }
        "mosaico_channel_join" => channel_mutation(state, caller, &args, true).await,
        "mosaico_channel_leave" => channel_mutation(state, caller, &args, false).await,
        "mosaico_dispatch" => {
            allow_only(&args, &["target", "workspace", "channels", "message"])?;
            let mut call = caller;
            for key in ["target", "workspace", "message"] {
                insert(&mut call, key, required(&args, key)?);
            }
            insert(&mut call, "channels", array(&args, "channels")?);
            super::session_dispatch::rpc_dispatch(state, &Value::Object(call)).await
        }
        "mosaico_channel_read" => anyhow::bail!("channel read uses the dedicated daemon stream"),
        other => anyhow::bail!("unsupported Pi agent tool {other:?}"),
    }
}

async fn send(
    state: &Arc<DaemonState>,
    caller: &Map<String, Value>,
    args: &Value,
) -> Result<Value> {
    allow_only(
        args,
        &[
            "message",
            "channel",
            "tags",
            "force",
            "attachments",
            "wait_seconds",
        ],
    )?;
    let mut call = caller.clone();
    insert(&mut call, "message", required(args, "message")?);
    insert(&mut call, "channel", optional(args, "channel"));
    insert(&mut call, "tags", array(args, "tags")?);
    insert(&mut call, "force", bool_or(args, "force", false));
    insert(&mut call, "wait_intent", args.get("wait_seconds").is_some());
    insert(&mut call, "attachments", attachments(args, caller)?);
    let sent = super::rpc_channel_send(state, &Value::Object(call)).await?;
    let Some(timeout_secs) = args.get("wait_seconds") else {
        return Ok(sent);
    };
    let timeout_secs = timeout_secs
        .as_u64()
        .filter(|value| *value > 0)
        .context("wait_seconds must be a positive integer")?;
    let mut wait = caller.clone();
    insert(&mut wait, "timeout_secs", timeout_secs);
    insert(
        &mut wait,
        "reply_to",
        sent["event_id"]
            .as_str()
            .context("send returned no event id")?,
    );
    insert(&mut wait, "from_pubkeys", sent["mentioned_pubkeys"].clone());
    insert(&mut wait, "from_labels", sent["mentioned_labels"].clone());
    Ok(
        json!({"send": sent, "wait": super::channel_wait::rpc_channel_wait(state, &Value::Object(wait)).await?}),
    )
}

async fn reply(
    state: &Arc<DaemonState>,
    caller: &Map<String, Value>,
    args: &Value,
) -> Result<Value> {
    allow_only(args, &["message_id", "message", "attachments"])?;
    let mut call = caller.clone();
    insert(&mut call, "id", required(args, "message_id")?);
    insert(&mut call, "message", required(args, "message")?);
    insert(&mut call, "attachments", attachments(args, caller)?);
    super::channel_send::rpc_channel_reply(state, &Value::Object(call)).await
}

async fn channel_mutation(
    state: &Arc<DaemonState>,
    mut caller: Map<String, Value>,
    args: &Value,
    join: bool,
) -> Result<Value> {
    allow_only(args, &["channel"])?;
    insert(&mut caller, "channel", required(args, "channel")?);
    if join {
        super::rpc_channel_join(state, &Value::Object(caller)).await
    } else {
        super::rpc_channel_leave(state, &Value::Object(caller)).await
    }
}

fn tool_ok(details: Value) -> Value {
    let text = serde_json::to_string_pretty(&details).unwrap_or_else(|_| details.to_string());
    json!({"content":[{"type":"text","text":text}],"details":details,"is_error":false})
}

fn tool_error(message: String) -> Value {
    json!({"content":[{"type":"text","text":message}],"details":{"error":message},"is_error":true})
}
