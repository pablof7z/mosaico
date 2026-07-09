use super::protocol::{notification, required_string, write_value, SharedWriter};
use anyhow::Result;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

const WHO_URI: &str = "tenex-edge://who";
const STATUS_PREFIX: &str = "tenex-edge://channels/status/";

#[derive(Clone, Default)]
pub(super) struct Subscriptions {
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
}

enum ResourceUri {
    Who,
    ChannelStatus(String),
}

pub(super) fn subscription_channel(uri: &str) -> Result<Option<String>> {
    Ok(match parse_uri(uri)? {
        ResourceUri::Who => None,
        ResourceUri::ChannelStatus(channel) => Some(channel),
    })
}

pub(super) fn event_updates_status_resource(item: &Value) -> bool {
    matches!(
        item.get("category").and_then(Value::as_str),
        Some("status" | "join" | "leave" | "turn" | "sess" | "proj" | "msg")
    )
}

pub(super) fn list() -> Value {
    let mut resources = vec![resource(
        WHO_URI,
        "who",
        "Current tenex-edge awareness snapshot",
    )];
    if let Some(channel) = super::super::channel_env() {
        resources.push(resource(
            &status_uri(&channel),
            "current-channel-status",
            "Live status snapshot for the current channel",
        ));
    }
    json!({ "resources": resources })
}

pub(super) fn templates() -> Value {
    json!({
        "resourceTemplates": [{
            "uriTemplate": "tenex-edge://channels/status/{channel}",
            "name": "channel-status",
            "title": "Channel Status",
            "description": "Current roster, activity, and fabric context for a tenex-edge channel.",
            "mimeType": "application/json"
        }]
    })
}

pub(super) async fn read(params: &Value) -> Result<Value> {
    let uri = required_string(params, "uri")?;
    let parsed = parse_uri(&uri)?;
    let value = match parsed {
        ResourceUri::Who => daemon_call("who", crate::cli::rpc_params(json!({}))).await?,
        ResourceUri::ChannelStatus(channel) => {
            daemon_call("who", operator_params(json!({ "channel": channel }))).await?
        }
    };
    let text = serde_json::to_string_pretty(&value)?;
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": text,
        }]
    }))
}

impl Subscriptions {
    pub(super) async fn add(&self, params: &Value, writer: SharedWriter) -> Result<()> {
        let uri = required_string(params, "uri")?;
        let channel = match parse_uri(&uri)? {
            ResourceUri::Who => None,
            ResourceUri::ChannelStatus(channel) => Some(channel),
        };
        let mut tasks = self.tasks.lock().await;
        if tasks.contains_key(&uri) {
            return Ok(());
        }
        tasks.insert(
            uri.clone(),
            tokio::spawn(run_subscription(uri, channel, writer)),
        );
        Ok(())
    }

    pub(super) async fn remove(&self, params: &Value) -> Result<()> {
        let uri = required_string(params, "uri")?;
        if let Some(task) = self.tasks.lock().await.remove(&uri) {
            task.abort();
        }
        Ok(())
    }

    pub(super) async fn shutdown(&self) {
        for (_, task) in self.tasks.lock().await.drain() {
            task.abort();
        }
    }
}

async fn run_subscription(uri: String, channel: Option<String>, writer: SharedWriter) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let writer_task = {
        let uri = uri.clone();
        let writer = writer.clone();
        tokio::spawn(async move {
            while rx.recv().await.is_some() {
                let note = notification("notifications/resources/updated", json!({ "uri": uri }));
                if write_value(&writer, &note).await.is_err() {
                    break;
                }
            }
        })
    };

    let params = json!({
        "channel": channel,
        "backfill": 0,
    });
    let stream_result = async {
        let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
        client
            .stream("tail", params, move |item| {
                if event_updates_status_resource(&item) {
                    let _ = tx.send(());
                }
            })
            .await
    }
    .await;
    if let Err(err) = stream_result {
        eprintln!("[tenex-edge mcp] subscription for {uri} ended: {err:#}");
    }
    writer_task.abort();
}

fn parse_uri(uri: &str) -> Result<ResourceUri> {
    if uri == WHO_URI {
        return Ok(ResourceUri::Who);
    }
    if let Some(channel) = uri.strip_prefix(STATUS_PREFIX) {
        let channel = channel.trim();
        if !channel.is_empty() {
            return Ok(ResourceUri::ChannelStatus(channel.to_string()));
        }
    }
    anyhow::bail!("unsupported tenex-edge MCP resource URI: {uri}")
}

fn resource(uri: &str, name: &str, description: &str) -> Value {
    json!({
        "uri": uri,
        "name": name,
        "title": name,
        "description": description,
        "mimeType": "application/json",
    })
}

fn status_uri(channel: &str) -> String {
    format!("{STATUS_PREFIX}{channel}")
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

async fn daemon_call(method: &str, params: Value) -> Result<Value> {
    let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
    client.call(method, params).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_resource_uris() {
        assert!(matches!(parse_uri(WHO_URI).unwrap(), ResourceUri::Who));
        assert!(matches!(
            parse_uri("tenex-edge://channels/status/root/task").unwrap(),
            ResourceUri::ChannelStatus(channel) if channel == "root/task"
        ));
    }

    #[test]
    fn status_resource_update_filter_ignores_profiles() {
        assert!(event_updates_status_resource(
            &json!({"category": "status"})
        ));
        assert!(!event_updates_status_resource(
            &json!({"category": "profile"})
        ));
    }
}
