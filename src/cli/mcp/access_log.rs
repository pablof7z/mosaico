use axum::http::HeaderMap;
use serde_json::{json, Value};

pub(super) fn log_http_event(
    kind: &str,
    headers: &HeaderMap,
    method: Option<&str>,
    params: &Value,
) {
    let mut event = serde_json::Map::new();
    event.insert("ts".into(), json!(crate::util::now_secs()));
    event.insert("kind".into(), json!(kind));
    if let Some(method) = method {
        event.insert("mcp_method".into(), json!(method));
    }
    add_param_shape(&mut event, params);
    event.insert("headers".into(), json!(selected_headers(headers)));
    eprintln!("[tenex-edge mcp access] {}", Value::Object(event));
}

fn add_param_shape(event: &mut serde_json::Map<String, Value>, params: &Value) {
    if let Some(tool) = params.get("name").and_then(Value::as_str) {
        event.insert("tool".into(), json!(tool));
    }
    if let Some(uri) = params.get("uri").and_then(Value::as_str) {
        event.insert("resource_uri".into(), json!(uri));
    }
    if let Some(protocol) = params.get("protocolVersion").and_then(Value::as_str) {
        event.insert("protocol_version".into(), json!(protocol));
    }
    if let Some(client) = params.get("clientInfo") {
        event.insert("client_info".into(), client.clone());
    }
    if let Some(arguments) = params.get("arguments").and_then(Value::as_object) {
        let mut keys = arguments.keys().cloned().collect::<Vec<_>>();
        keys.sort();
        event.insert("argument_keys".into(), json!(keys));
        add_arg(event, arguments, "session");
        add_arg(event, arguments, "channel");
        add_arg(event, arguments, "channel");
    }
}

fn add_arg(
    event: &mut serde_json::Map<String, Value>,
    args: &serde_json::Map<String, Value>,
    key: &str,
) {
    if let Some(value) = args.get(key).and_then(Value::as_str) {
        event.insert(format!("argument_{key}"), json!(value));
    }
}

fn selected_headers(headers: &HeaderMap) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for (name, value) in headers {
        let key = name.as_str().to_ascii_lowercase();
        if matches!(
            key.as_str(),
            "authorization" | "cookie" | "set-cookie" | "proxy-authorization"
        ) {
            continue;
        }
        out.insert(key, json!(value.to_str().unwrap_or("<non-utf8>")));
    }
    out
}
