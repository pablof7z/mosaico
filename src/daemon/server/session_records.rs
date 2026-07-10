use nostr_sdk::prelude::Keys;
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone)]
pub(super) struct HostedAgent {
    pub(super) keys: Keys,
}

pub(super) struct SessionHandle {
    pub(super) cancel: Arc<Notify>,
}

/// Metadata tracked per live peer session for join/leave derivation.
#[derive(Clone)]
pub(super) struct PeerTracked {
    pub(super) first_seen: u64,
    pub(super) channel: String,
    pub(super) slug: String,
    pub(super) host: String,
}

pub(super) type StatusTailKey = (String, String, String);
pub(super) type StatusTailSnapshot = (String, bool);
