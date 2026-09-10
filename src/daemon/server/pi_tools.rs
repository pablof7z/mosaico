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

async fn execute(state: &Arc<DaemonState>, params: &Value) -> Result<String> {
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
            let v = super::rpc_my_session(state, &Value::Object(caller))?;
            Ok(v["fabric"].as_str().unwrap_or_default().to_string())
        }
        "mosaico_wait" => {
            allow_only(&args, &["seconds", "channels", "from"])?;
            let mut call = caller;
            insert(&mut call, "timeout_secs", required_u64(&args, "seconds")?);
            insert(&mut call, "channels", array(&args, "channels")?);
            insert(&mut call, "from_labels", array(&args, "from")?);
            let v = super::channel_wait::rpc_channel_wait(state, &Value::Object(call)).await?;
            crate::channel_render::format_wait_result(&v, crate::util::now_secs())
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
            let v = super::rpc_channel_list(state, &Value::Object(call))?;
            Ok(crate::channel_render::format_channel_list(&v))
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
            let v = super::channel_search::rpc_channel_search(state, &Value::Object(call))?;
            crate::channel_render::format_search_response(&v)
        }
        "mosaico_channel_read" => {
            allow_only(&args, &["channel", "id", "since", "limit"])?;
            let mut call = caller;
            insert(&mut call, "channel", optional(&args, "channel"));
            insert(&mut call, "id", optional(&args, "id"));
            insert(&mut call, "since", optional(&args, "since"));
            insert(&mut call, "limit", optional(&args, "limit"));
            insert(&mut call, "tail", true);
            insert(&mut call, "live", false);
            let rows = super::channel_read_tail::channel_read_snapshot(state, &Value::Object(call))
                .await?;
            let text = rows
                .iter()
                .map(|row| crate::channel_render::render_channel_read_row(row, false))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(text)
        }
        "mosaico_send" => send(state, &caller, &args).await,
        "mosaico_reply" => reply(state, &caller, &args).await,
        "mosaico_react" => {
            allow_only(&args, &["id", "emoji"])?;
            let mut call = caller;
            insert(&mut call, "id", required(&args, "id")?);
            insert(&mut call, "emoji", required(&args, "emoji")?);
            let v = super::channel_send::rpc_channel_react(state, &Value::Object(call)).await?;
            let id = v["id"].as_str().unwrap_or("?");
            let target = v["target"].as_str().unwrap_or("?");
            let emoji = v["emoji"].as_str().unwrap_or("?");
            Ok(format!("reacted {emoji} {id} to {target}"))
        }
        "mosaico_channel_create" => {
            allow_only(&args, &["channel", "about", "agents"])?;
            let mut call = caller;
            insert(&mut call, "channel", required(&args, "channel")?);
            insert(&mut call, "about", required(&args, "about")?);
            insert(&mut call, "agents", agent_specs(&args)?);
            let v = super::rpc_channel_create(state, &Value::Object(call)).await?;
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
        "mosaico_channel_join" => channel_mutation(state, caller, &args, true).await,
        "mosaico_channel_leave" => channel_mutation(state, caller, &args, false).await,
        "mosaico_dispatch" => {
            allow_only(&args, &["target", "workspace", "channels", "message"])?;
            let mut call = caller;
            for key in ["target", "workspace", "message"] {
                insert(&mut call, key, required(&args, key)?);
            }
            insert(&mut call, "channels", array(&args, "channels")?);
            let v = super::session_dispatch::rpc_dispatch(state, &Value::Object(call)).await?;
            let dispatch_id =
                crate::idref::event_short_id(v["dispatch_event_id"].as_str().unwrap_or("?"));
            let message_id =
                crate::idref::event_short_id(v["message_event_id"].as_str().unwrap_or("?"));
            let workspace = v["workspace"].as_str().unwrap_or("?");
            let route = v["route_channel"].as_str().unwrap_or("?");
            Ok(format!(
                "dispatched {dispatch_id} in {workspace} via {route}; message {message_id}"
            ))
        }
        other => anyhow::bail!("unsupported Pi agent tool {other:?}"),
    }
}

async fn send(
    state: &Arc<DaemonState>,
    caller: &Map<String, Value>,
    args: &Value,
) -> Result<String> {
    allow_only(
        args,
        &[
            "message",
            "channel",
            "tags",
            "force",
            "attachments",
            "seconds",
        ],
    )?;
    let mut call = caller.clone();
    insert(&mut call, "message", required(args, "message")?);
    insert(&mut call, "channel", optional(args, "channel"));
    insert(&mut call, "tags", array(args, "tags")?);
    insert(&mut call, "force", bool_or(args, "force", false));
    insert(&mut call, "wait_intent", args.get("seconds").is_some());
    insert(&mut call, "attachments", attachments(args, caller)?);
    let sent = super::rpc_channel_send(state, &Value::Object(call)).await?;
    let id = sent["id"].as_str().unwrap_or("?");
    let notice_lines =
        crate::channel_render::send_notice_lines(&sent, crate::channel_render::NoticeHint::Tool)?;
    let Some(timeout_secs) = args.get("seconds") else {
        let mut lines = notice_lines;
        lines.push(format!("sent chat {id}"));
        return Ok(lines.join("\n"));
    };
    let timeout_secs = timeout_secs
        .as_u64()
        .filter(|v| *v > 0)
        .context("seconds must be a positive integer")?;
    let mut wait = caller.clone();
    insert(&mut wait, "timeout_secs", timeout_secs);
    insert(
        &mut wait,
        "reply_to",
        sent["id"].as_str().context("send returned no id")?,
    );
    let wait_result = super::channel_wait::rpc_channel_wait(state, &Value::Object(wait)).await?;
    let wait_text =
        crate::channel_render::format_wait_result(&wait_result, crate::util::now_secs())?;
    let mut lines = vec![format!("sent chat {id}")];
    lines.extend(notice_lines);
    lines.push(wait_text);
    Ok(lines.join("\n"))
}

async fn reply(
    state: &Arc<DaemonState>,
    caller: &Map<String, Value>,
    args: &Value,
) -> Result<String> {
    allow_only(args, &["id", "message", "attachments"])?;
    let mut call = caller.clone();
    insert(&mut call, "id", required(args, "id")?);
    insert(&mut call, "message", required(args, "message")?);
    insert(&mut call, "attachments", attachments(args, caller)?);
    let v = super::channel_send::rpc_channel_reply(state, &Value::Object(call)).await?;
    let id = v["id"].as_str().unwrap_or("?");
    let notice_lines =
        crate::channel_render::send_notice_lines(&v, crate::channel_render::NoticeHint::Tool)?;
    let mut lines = vec![format!("sent reply {id}")];
    lines.extend(notice_lines);
    Ok(lines.join("\n"))
}

async fn channel_mutation(
    state: &Arc<DaemonState>,
    mut caller: Map<String, Value>,
    args: &Value,
    join: bool,
) -> Result<String> {
    allow_only(args, &["channel"])?;
    insert(&mut caller, "channel", required(args, "channel")?);
    if join {
        let v = super::rpc_channel_join(state, &Value::Object(caller)).await?;
        let channel = v["channel"].as_str().unwrap_or("?");
        let mut text = format!("joined channel {channel}");
        if let Some(notice) = v["history_notice"].as_str().filter(|n| !n.is_empty()) {
            text.push('\n');
            text.push_str(notice);
        }
        Ok(text)
    } else {
        let v = super::rpc_channel_leave(state, &Value::Object(caller)).await?;
        let channel = v["channel"].as_str().unwrap_or("?");
        Ok(format!("left channel {channel}"))
    }
}

fn tool_ok(text: String) -> Value {
    json!({"content":[{"type":"text","text":text}],"is_error":false})
}

fn tool_error(message: String) -> Value {
    json!({"content":[{"type":"text","text":message}],"is_error":true})
}
