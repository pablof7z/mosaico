//! Wire protocol for the per-machine daemon (newline-delimited JSON over a UDS).
//!
//! Framing: one JSON object per line. A request carries an `id`, a `method`, and
//! `params`. A response carries the same `id` and either `ok` (single result),
//! `error`, or — for streaming verbs (`tail`) — zero or more `item` frames
//! terminated by an `end` frame. The handshake (`hello`/`welcome`) carries a
//! protocol-version integer so a newer client can detect an older daemon and ask
//! it to exit + re-exec rather than speak a stale protocol.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// The compiled-in protocol version, bumped when client and daemon RPC
/// contracts must agree.
const PROTOCOL_VERSION_BASE: u32 = 64;

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

/// Maximum time a spawning client waits for a daemon to become handshake-ready.
pub(crate) const DAEMON_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Per-connect/read timeout for the daemon hello/welcome handshake.
pub(crate) const DAEMON_HANDSHAKE_IO_TIMEOUT: Duration = Duration::from_secs(2);
/// Short grace period after asking an older daemon to exit before respawning.
pub(crate) const DAEMON_RESPAWN_GRACE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandshakeDecision {
    Ready,
    AskOlderDaemonToExit,
    DaemonTooNew {
        daemon_protocol: u32,
        client_protocol: u32,
    },
}

fn compare_protocols(daemon_protocol: u32, client_protocol: u32) -> HandshakeDecision {
    match daemon_protocol.cmp(&client_protocol) {
        std::cmp::Ordering::Equal => HandshakeDecision::Ready,
        std::cmp::Ordering::Less => HandshakeDecision::AskOlderDaemonToExit,
        std::cmp::Ordering::Greater => HandshakeDecision::DaemonTooNew {
            daemon_protocol,
            client_protocol,
        },
    }
}

pub(crate) fn handshake_decision(daemon_protocol: u32) -> HandshakeDecision {
    compare_protocols(daemon_protocol, protocol_version())
}

pub(crate) fn client_hello() -> Hello {
    Hello {
        protocol: protocol_version(),
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

pub(crate) fn please_exit() -> PleaseExit {
    PleaseExit {
        protocol: protocol_version(),
    }
}

pub(crate) fn daemon_too_new_message(daemon_protocol: u32, client_protocol: u32) -> String {
    format!(
        "daemon protocol {daemon_protocol} is newer than this binary's {client_protocol} — restart your tenex-edge session (or reinstall)"
    )
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

// The `who` snapshot DTO is `crate::who_snapshot::WhoSnapshot` (Serialize/
// Deserialize): the daemon serializes the exact struct the CLI renderers
// consume, so `who` output is byte-identical by construction without making
// the daemon depend on CLI presentation modules.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_compare_covers_equal_older_and_newer_daemons() {
        assert_eq!(compare_protocols(62, 62), HandshakeDecision::Ready);
        assert_eq!(
            compare_protocols(61, 62),
            HandshakeDecision::AskOlderDaemonToExit
        );
        assert_eq!(
            compare_protocols(63, 62),
            HandshakeDecision::DaemonTooNew {
                daemon_protocol: 63,
                client_protocol: 62
            }
        );
    }

    #[test]
    fn daemon_too_new_message_matches_client_paths() {
        let msg = daemon_too_new_message(58, 57);
        assert!(msg.contains("daemon protocol 58 is newer"));
        assert!(msg.contains("restart your tenex-edge session"));
    }
}
