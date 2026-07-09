use anyhow::Result;
use clap::Args;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;

mod access_log;
mod auth;
mod auth_nip07;
mod auth_routes;
mod auth_support;
mod auth_types;
mod catalog;
mod http;
mod protocol;
mod resources;
mod tools;

use protocol::{
    error, result, write_value, Message, SharedWriter, INVALID_PARAMS, INVALID_REQUEST,
    METHOD_NOT_FOUND, PARSE_ERROR,
};

const MCP_VERSION: &str = "2025-11-25";

#[derive(Debug, Clone, Args)]
pub(super) struct McpArgs {
    /// Serve MCP over HTTP instead of stdio.
    #[arg(long)]
    http: bool,
    /// HTTP bind address. Keep localhost for tunneled development.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// HTTP bind port. Use 0 to let the OS pick a free port.
    #[arg(long, default_value_t = 8765)]
    port: u16,
    /// HTTP MCP endpoint path.
    #[arg(long, default_value = "/mcp")]
    path: String,
    /// Require OAuth for HTTP MCP requests.
    #[arg(long)]
    oauth: bool,
    /// Public HTTPS origin for OAuth metadata, e.g. https://edge.f7z.io.
    #[arg(long)]
    public_url: Option<String>,
}

pub(super) async fn mcp(args: McpArgs) -> Result<()> {
    if args.http {
        return http::serve(args).await;
    }
    stdio().await
}

async fn stdio() -> Result<()> {
    let writer: SharedWriter = Arc::new(Mutex::new(tokio::io::stdout()));
    let subscriptions = resources::Subscriptions::default();
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();

    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message = match Message::parse(line) {
            Ok(message) => message,
            Err(err) => {
                let response = error(Value::Null, PARSE_ERROR, format!("{err:#}"));
                write_value(&writer, &response).await?;
                continue;
            }
        };
        if message.is_notification() {
            handle_notification(message.method.as_deref().unwrap_or_default()).await;
            continue;
        }
        let Some(id) = message.id.clone() else {
            let response = error(Value::Null, INVALID_REQUEST, "request id is required");
            write_value(&writer, &response).await?;
            continue;
        };
        let response = handle_request(&message, id, &subscriptions, writer.clone()).await;
        write_value(&writer, &response).await?;
    }

    subscriptions.shutdown().await;
    Ok(())
}

async fn handle_request(
    message: &Message,
    id: Value,
    subscriptions: &resources::Subscriptions,
    writer: SharedWriter,
) -> Value {
    let Some(method) = message.method.as_deref() else {
        return error(id, INVALID_REQUEST, "missing method");
    };
    match method {
        "initialize" => result(id, initialize(&message.params)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, tools::list()),
        "tools/call" => match tools::call(&message.params).await {
            Ok(value) => result(id, value),
            Err(err) => error(id, INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/list" => result(id, resources::list()),
        "resources/templates/list" => result(id, resources::templates()),
        "resources/read" => match resources::read(&message.params).await {
            Ok(value) => result(id, value),
            Err(err) => error(id, INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/subscribe" => match subscriptions.subscribe(&message.params, writer).await {
            Ok(()) => result(id, json!({})),
            Err(err) => error(id, INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/unsubscribe" => match subscriptions.unsubscribe(&message.params).await {
            Ok(()) => result(id, json!({})),
            Err(err) => error(id, INVALID_PARAMS, format!("{err:#}")),
        },
        other => error(id, METHOD_NOT_FOUND, format!("unknown method: {other}")),
    }
}

async fn handle_notification(method: &str) {
    match method {
        "notifications/initialized" | "notifications/cancelled" => {}
        _ => {}
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(MCP_VERSION);
    let protocol = match requested {
        "2025-11-25" | "2025-06-18" | "2024-11-05" => requested,
        _ => MCP_VERSION,
    };
    json!({
        "protocolVersion": protocol,
        "capabilities": {
            "resources": { "subscribe": true },
            "tools": {},
        },
        "serverInfo": {
            "name": "tenex-edge",
            "title": "tenex-edge",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Use tenex-edge resources for channel awareness and tenex-edge tools for channel chat and membership operations.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_resources_and_tools() {
        let value = initialize(&json!({ "protocolVersion": "2025-11-25" }));

        assert_eq!(value["protocolVersion"], "2025-11-25");
        assert_eq!(value["capabilities"]["resources"]["subscribe"], true);
        assert!(value["capabilities"]["tools"].is_object());
    }
}
