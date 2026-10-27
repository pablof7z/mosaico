//! Tiny raw NIP-01 client used to inspect a relay independently of Mosaico.

#![allow(dead_code)]

use anyhow::{Context, Result};
use nostr::event::{Event, EventBuilder, EventId, FinalizeEvent as _};
use nostr::filter::Filter;
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

pub struct NostrRelayClient {
    socket: Mutex<tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>>,
    relay: String,
    keys: Keys,
}

#[derive(Debug)]
pub struct WriteAck {
    pub val: EventId,
    pub success: BTreeSet<String>,
    pub failed: BTreeMap<String, String>,
}

impl NostrRelayClient {
    pub async fn connect(keys: Keys, relay: &str) -> Result<Self> {
        let (socket, _) = tungstenite::connect(relay).context("connect test relay")?;
        Ok(Self {
            socket: Mutex::new(socket),
            relay: relay.to_string(),
            keys,
        })
    }

    pub fn register_identity(&mut self, _keys: &Keys) -> Result<()> {
        // The witness signs builders with its fixed key and accepts pre-signed events.
        // This is a no-op for tests that send pre-signed events via send_event().
        Ok(())
    }

    pub async fn send_event_builder(&self, builder: EventBuilder) -> Result<WriteAck> {
        let event = builder.finalize(&self.keys).context("sign test event")?;
        self.send_event(&event).await
    }

    pub async fn send_event(&self, event: &Event) -> Result<WriteAck> {
        let event_id = event.id;
        let mut socket = self.socket.lock().expect("test relay socket poisoned");
        socket.send(tungstenite::Message::Text(
            serde_json::json!(["EVENT", event]).to_string().into(),
        ))?;
        loop {
            let message = socket.read()?;
            let tungstenite::Message::Text(text) = message else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&text)?;
            let Some(frame) = value.as_array() else {
                continue;
            };
            if frame.first().and_then(serde_json::Value::as_str) != Some("OK")
                || frame.get(1).and_then(serde_json::Value::as_str) != Some(&event_id.to_hex())
            {
                continue;
            }
            let accepted = frame
                .get(2)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let message = frame
                .get(3)
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            return Ok(WriteAck {
                val: event_id,
                success: if accepted {
                    BTreeSet::from([self.relay.clone()])
                } else {
                    BTreeSet::new()
                },
                failed: if accepted {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([(self.relay.clone(), message.to_string())])
                },
            });
        }
    }

    pub async fn fetch_events(&self, filter: Filter, timeout: Duration) -> Result<Vec<Event>> {
        let _ = timeout;
        let subscription = format!("mosaico-test-{}", std::process::id());
        let mut socket = self.socket.lock().expect("test relay socket poisoned");
        socket.send(tungstenite::Message::Text(
            serde_json::json!(["REQ", subscription, filter])
                .to_string()
                .into(),
        ))?;
        let mut events = Vec::new();
        loop {
            let tungstenite::Message::Text(text) = socket.read()? else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&text)?;
            let Some(frame) = value.as_array() else {
                continue;
            };
            match frame.first().and_then(serde_json::Value::as_str) {
                Some("EVENT")
                    if frame.get(1).and_then(serde_json::Value::as_str) == Some(&subscription) =>
                {
                    events.push(serde_json::from_value(frame[2].clone())?);
                }
                Some("EOSE")
                    if frame.get(1).and_then(serde_json::Value::as_str) == Some(&subscription) =>
                {
                    break
                }
                _ => {}
            }
        }
        Ok(events)
    }

    pub async fn disconnect(&self) {
        // Nostr test client cleanup happens on drop; no explicit shutdown needed.
    }
}
