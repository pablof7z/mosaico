use super::super::*;
use crate::daemon::protocol::Request;
use nostr::Keys;

#[path = "tests/readiness.rs"]
mod readiness;
#[path = "tests/status.rs"]
mod status;

const RELAY: &str = "wss://relay.example.com";
