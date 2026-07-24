//! Exact NIP-42 wire assertions shared by the strict relay connection loop.

use std::collections::BTreeSet;

use nostr::{Event, JsonUtil, Kind, PublicKey, RelayMessage, RelayUrl, Timestamp};
use tungstenite::{Message, WebSocket};

pub(super) fn send(ws: &mut WebSocket<std::net::TcpStream>, message: RelayMessage<'_>) {
    ws.send(Message::text(message.as_json()))
        .expect("send AUTH relay frame");
}

pub(super) fn validate_auth_event(
    event: &Event,
    allowed: &BTreeSet<PublicKey>,
    relay: &RelayUrl,
    challenge: &str,
) -> Result<(), String> {
    if !allowed.contains(&event.pubkey) {
        return Err("AUTH pubkey is outside the harness allowlist".into());
    }
    if event.kind != Kind::Authentication || !event.content.is_empty() {
        return Err("AUTH must be an empty kind:22242 event".into());
    }
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    let expected = vec![
        vec!["challenge".to_string(), challenge.to_string()],
        vec!["relay".to_string(), relay.to_string()],
    ];
    if tags != expected {
        return Err(format!(
            "wrong AUTH tags: expected {expected:?}, got {tags:?}"
        ));
    }
    event
        .verify()
        .map_err(|error| format!("invalid AUTH signature: {error}"))?;
    if Timestamp::now()
        .as_secs()
        .abs_diff(event.created_at.as_secs())
        > 10
    {
        return Err("AUTH timestamp is stale".into());
    }
    Ok(())
}
