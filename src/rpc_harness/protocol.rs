//! Wire types for the two stdio JSON-RPC dialects (ACP + codex app-server).
//!
//! Kept deliberately thin: params are built as `serde_json::Value` at the call
//! sites; this module names the shared framing + the small set of result shapes
//! the engine decodes.

use serde::{Deserialize, Serialize};

/// Which JSON-RPC dialect a spawned child speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// Agent Client Protocol (OpenCode native, Claude via adapter).
    Acp,
    /// Codex `app-server` dialect.
    AppServer,
    /// Pi's command/response/event JSONL protocol (not JSON-RPC 2.0).
    PiRpc,
}

pub(crate) const PI_TURN_KEY: &str = "pi-agent-turn";

/// Why a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    Cancelled,
    MaxTokens,
    Refusal,
    Other,
}

impl StopReason {
    pub fn from_wire(s: &str) -> Self {
        match s {
            "end_turn" => StopReason::EndTurn,
            "cancelled" | "canceled" => StopReason::Cancelled,
            "max_tokens" | "max_token_usage" => StopReason::MaxTokens,
            "refusal" => StopReason::Refusal,
            _ => StopReason::Other,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::Cancelled => "cancelled",
            Self::MaxTokens => "max_tokens",
            Self::Refusal => "refusal",
            Self::Other => "other",
        }
    }
}

/// A classified inbound JSON-RPC frame.
pub enum Inbound {
    /// Response to one of our requests (has `id` + result/error).
    Response {
        id: i64,
        result: Result<serde_json::Value, RpcErrorObject>,
    },
    /// Agent->client request (has `id` + `method`); we must reply.
    Request {
        id: serde_json::Value,
        method: String,
        params: serde_json::Value,
    },
    /// Notification (has `method`, no `id`).
    Notification {
        method: String,
        params: serde_json::Value,
    },
    /// Unparseable / uninteresting line.
    Other,
}

/// The JSON-RPC `error` object.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Classify a decoded JSON value into an [`Inbound`].
pub fn classify(v: serde_json::Value) -> Inbound {
    classify_for(Dialect::Acp, v)
}

pub fn classify_for(dialect: Dialect, v: serde_json::Value) -> Inbound {
    if dialect == Dialect::PiRpc {
        return classify_pi(v);
    }
    let obj = match v.as_object() {
        Some(o) => o,
        None => return Inbound::Other,
    };
    let has_method = obj.contains_key("method");
    let id = obj.get("id").cloned();

    if has_method {
        let method = obj
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string();
        let params = obj
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        match id {
            Some(id) if !id.is_null() => Inbound::Request { id, method, params },
            _ => Inbound::Notification { method, params },
        }
    } else if let Some(id) = id.as_ref().and_then(|v| v.as_i64()) {
        if let Some(err) = obj.get("error") {
            let eo: RpcErrorObject =
                serde_json::from_value(err.clone()).unwrap_or(RpcErrorObject {
                    code: -1,
                    message: err.to_string(),
                    data: None,
                });
            Inbound::Response {
                id,
                result: Err(eo),
            }
        } else {
            let result = obj
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            Inbound::Response {
                id,
                result: Ok(result),
            }
        }
    } else {
        Inbound::Other
    }
}

fn classify_pi(v: serde_json::Value) -> Inbound {
    let Some(object) = v.as_object() else {
        return Inbound::Other;
    };
    let kind = object.get("type").and_then(serde_json::Value::as_str);
    if kind == Some("response") {
        let Some(id) = object
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(|id| id.parse::<i64>().ok())
        else {
            return Inbound::Other;
        };
        let result = if object.get("success").and_then(serde_json::Value::as_bool) == Some(true) {
            Ok(object
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Null))
        } else {
            Err(RpcErrorObject {
                code: -1,
                message: object
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Pi RPC command failed")
                    .to_string(),
                data: None,
            })
        };
        return Inbound::Response { id, result };
    }
    match kind {
        Some(method) => Inbound::Notification {
            method: method.to_string(),
            params: v,
        },
        None => Inbound::Other,
    }
}

/// A high-level notification surfaced to the caller for transcript/turn tracking.
#[derive(Debug, Clone)]
pub struct SessionUpdate {
    pub method: String,
    pub params: serde_json::Value,
}
