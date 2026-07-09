use super::protocol::{error, notification, result, Message, INVALID_REQUEST, METHOD_NOT_FOUND};
use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use super::access_log::log_http_event;

#[derive(Clone)]
pub(super) struct HttpState {
    subscriptions: HttpSubscriptions,
    pub(super) auth: Option<super::auth::AuthState>,
}

#[derive(Clone)]
pub(super) struct HttpSubscriptions {
    tasks: Arc<Mutex<HashMap<String, JoinHandle<()>>>>,
    tx: broadcast::Sender<Value>,
}

impl Default for HttpSubscriptions {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(128);
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
            tx,
        }
    }
}

pub(super) async fn serve(args: super::McpArgs) -> Result<()> {
    let addr = SocketAddr::new(args.host.parse::<IpAddr>()?, args.port);
    let path = normalize_path(&args.path)?;
    let auth = auth_state(&args)?;
    let state = HttpState {
        subscriptions: HttpSubscriptions::default(),
        auth,
    };
    let app = Router::new()
        .route("/", get(root_health))
        .merge(super::auth_routes::routes())
        .route(&path, post(post_mcp).get(get_mcp).options(options_mcp))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    eprintln!(
        "[tenex-edge] MCP HTTP listening on http://{}:{}{}",
        local.ip(),
        local.port(),
        path
    );
    eprintln!(
        "[tenex-edge] ChatGPT requires HTTPS; tunnel this endpoint and use https://<host>{path}"
    );
    axum::serve(listener, app)
        .await
        .context("serving MCP HTTP endpoint")
}

async fn post_mcp(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(value): Json<Value>,
) -> Response {
    let message = match serde_json::from_value::<Message>(value) {
        Ok(message) => message,
        Err(err) => {
            log_http_event("post_parse_error", &headers, None, &Value::Null);
            return json_response(
                StatusCode::BAD_REQUEST,
                error(
                    Value::Null,
                    super::protocol::PARSE_ERROR,
                    format!("{err:#}"),
                ),
            );
        }
    };
    log_http_event("post", &headers, message.method.as_deref(), &message.params);
    let Some(method) = message.method.as_deref() else {
        return StatusCode::ACCEPTED.into_response();
    };
    if let Err(response) = state.require_auth(&headers, required_scope(method, &message.params)) {
        return *response;
    }
    if message.is_notification() {
        return StatusCode::ACCEPTED.into_response();
    }
    let Some(id) = message.id.clone() else {
        return json_response(
            StatusCode::BAD_REQUEST,
            error(Value::Null, INVALID_REQUEST, "request id is required"),
        );
    };
    let response = dispatch_http(&state, method, &message.params, id).await;
    json_response(StatusCode::OK, response)
}

async fn root_health(headers: HeaderMap) -> impl IntoResponse {
    log_http_event("root", &headers, None, &Value::Null);
    Json(json!({ "ok": true, "name": "tenex-edge MCP", "mcp": "/mcp" }))
}

async fn get_mcp(State(state): State<HttpState>, headers: HeaderMap) -> impl IntoResponse {
    log_http_event("sse_get", &headers, None, &Value::Null);
    if let Err(response) = state.require_auth(&headers, "tenex:read") {
        return *response;
    }
    let rx = state.subscriptions.tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|value| match value {
        Ok(value) => Some(Ok::<_, Infallible>(
            Event::default().data(value.to_string()),
        )),
        Err(_) => None,
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn options_mcp(headers: HeaderMap) -> Response {
    log_http_event("options", &headers, None, &Value::Null);
    (StatusCode::NO_CONTENT, cors_headers()).into_response()
}

impl HttpState {
    fn require_auth(
        &self,
        headers: &HeaderMap,
        scope: &str,
    ) -> std::result::Result<(), Box<Response>> {
        match &self.auth {
            Some(auth) => auth.verify(headers, scope).map(|_| ()),
            None => Ok(()),
        }
    }
}

fn auth_state(args: &super::McpArgs) -> Result<Option<super::auth::AuthState>> {
    if !args.oauth {
        return Ok(None);
    }
    let public_url = args
        .public_url
        .clone()
        .context("--public-url is required with --oauth")?;
    super::auth::AuthState::new(public_url).map(Some)
}

fn required_scope(method: &str, params: &Value) -> &'static str {
    if method == "tools/call" {
        let tool = params
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if super::catalog::requires_write(tool) {
            return "tenex:write";
        }
    }
    "tenex:read"
}

async fn dispatch_http(state: &HttpState, method: &str, params: &Value, id: Value) -> Value {
    match method {
        "initialize" => result(id, super::initialize(params)),
        "ping" => result(id, json!({})),
        "tools/list" => result(id, super::tools::list()),
        "tools/call" => match super::tools::call(params).await {
            Ok(value) => result(id, value),
            Err(err) => error(id, super::protocol::INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/list" => result(id, super::resources::list()),
        "resources/templates/list" => result(id, super::resources::templates()),
        "resources/read" => match super::resources::read(params).await {
            Ok(value) => result(id, value),
            Err(err) => error(id, super::protocol::INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/subscribe" => match state.subscriptions.add(params).await {
            Ok(()) => result(id, json!({})),
            Err(err) => error(id, super::protocol::INVALID_PARAMS, format!("{err:#}")),
        },
        "resources/unsubscribe" => match state.subscriptions.remove(params).await {
            Ok(()) => result(id, json!({})),
            Err(err) => error(id, super::protocol::INVALID_PARAMS, format!("{err:#}")),
        },
        other => error(id, METHOD_NOT_FOUND, format!("unknown method: {other}")),
    }
}

impl HttpSubscriptions {
    async fn add(&self, params: &Value) -> Result<()> {
        let uri = super::protocol::required_string(params, "uri")?;
        let channel = super::resources::subscription_channel(&uri)?;
        let mut tasks = self.tasks.lock().await;
        if tasks.contains_key(&uri) {
            return Ok(());
        }
        tasks.insert(
            uri.clone(),
            tokio::spawn(run_subscription(uri, channel, self.tx.clone())),
        );
        Ok(())
    }

    async fn remove(&self, params: &Value) -> Result<()> {
        let uri = super::protocol::required_string(params, "uri")?;
        if let Some(task) = self.tasks.lock().await.remove(&uri) {
            task.abort();
        }
        Ok(())
    }
}

async fn run_subscription(uri: String, channel: Option<String>, tx: broadcast::Sender<Value>) {
    let params = json!({ "channel": channel, "backfill": 0 });
    let note_uri = uri.clone();
    let stream_result = async {
        let mut client = crate::daemon::client::Client::connect_or_spawn().await?;
        client
            .stream("tail", params, move |item| {
                if super::resources::event_updates_status_resource(&item) {
                    let note = notification(
                        "notifications/resources/updated",
                        json!({ "uri": note_uri.clone() }),
                    );
                    let _ = tx.send(note);
                }
            })
            .await
    }
    .await;
    if let Err(err) = stream_result {
        eprintln!("[tenex-edge mcp] HTTP subscription ended: {err:#}");
    }
}

fn normalize_path(path: &str) -> Result<String> {
    let path = path.trim();
    if path.is_empty() {
        anyhow::bail!("MCP HTTP path must not be empty");
    }
    Ok(if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    })
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, cors_headers(), Json(body)).into_response()
}

fn cors_headers() -> [(header::HeaderName, HeaderValue); 3] {
    [
        (
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static("*"),
        ),
        (
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type, accept, authorization, mcp-protocol-version"),
        ),
        (
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, OPTIONS"),
        ),
    ]
}
