//! Wire protocol for the per-machine daemon (newline-delimited JSON over a UDS).
//!
//! Framing: one JSON object per line. A request carries an `id`, a `method`, and
//! `params`. A response carries the same `id` and either `ok` (single result),
//! `error`, or — for streaming verbs (`tail`) — zero or more `item` frames
//! terminated by an `end` frame. The handshake (`hello`/`welcome`) carries a
//! protocol-version integer so a newer client can detect an older daemon and ask
//! it to exit + re-exec rather than speak a stale protocol.

use serde::{Deserialize, Serialize};

/// The compiled-in protocol version, bumped on any breaking RPC change.
const PROTOCOL_VERSION_BASE: u32 = 4;

/// Effective protocol version. A client refuses to talk to a daemon whose
/// protocol differs (older daemon → ask it to exit & respawn; newer daemon →
/// tell the human to restart the session). `$TENEX_EDGE_PROTOCOL` overrides it
/// for tests that need to simulate a binary upgrade (a newer client meeting an
/// older daemon) without two binaries.
pub fn protocol_version() -> u32 {
    std::env::var("TENEX_EDGE_PROTOCOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(PROTOCOL_VERSION_BASE)
}

// ── handshake ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub client_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol: u32,
    pub daemon_version: String,
}

// ── requests ───────────────────────────────────────────────────────────────

/// A single RPC request line: `{"id":1,"method":"who","params":{…}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Control frame: a newer client asking an older daemon to exit so it can be
/// re-spawned at the new binary's protocol version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PleaseExit {
    pub protocol: u32,
}

// ── responses ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

/// One response frame. Exactly one of the option fields is set.
///   - `ok`:    single terminal result for a one-shot verb.
///   - `error`: terminal error.
///   - `item`:  one element of a streaming response (`tail`).
///   - `end`:   terminates a streaming response.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<bool>,
}

impl Response {
    pub fn ok(id: u64, val: serde_json::Value) -> Self {
        Response {
            id,
            ok: Some(val),
            ..Default::default()
        }
    }
    pub fn err(id: u64, code: &str, message: impl Into<String>) -> Self {
        Response {
            id,
            error: Some(RpcError {
                code: code.into(),
                message: message.into(),
            }),
            ..Default::default()
        }
    }
    pub fn item(id: u64, val: serde_json::Value) -> Self {
        Response {
            id,
            item: Some(val),
            ..Default::default()
        }
    }
    pub fn end(id: u64) -> Self {
        Response {
            id,
            end: Some(true),
            ..Default::default()
        }
    }
}

/// Error code used when the daemon's protocol is older than the client's; the
/// client treats this as "exit-and-respawn", not a hard failure.
pub const ERR_PROTOCOL_SKEW: &str = "protocol_skew";

// The `who` snapshot DTO is `crate::cli::WhoSnapshot` itself (Serialize/
// Deserialize): the daemon serializes the exact struct the CLI renderers
// consume, so `who` output is byte-identical by construction.
