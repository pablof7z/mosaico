//! The daemon's sole Nostr transport and raw-event database owner.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use nostr::prelude::{Event, Filter, RelayMessage, RelayUrl, SubscriptionId, Timestamp};
use nostr_memory::MemoryDatabase;
use nostr_sdk::client::{Client, ClientNotification};
use tokio::sync::watch;
use tokio_stream::StreamExt;

pub(crate) const CONTENT_EVENT_LIMIT: usize = 4_096;
pub(crate) const GROUP_RECORD_LIMIT: usize = 1_024;
pub(crate) const PROFILE_EVENT_LIMIT: usize = 512;
pub(crate) const REFERENCE_EVENT_LIMIT: usize = 64;
const DATABASE_HEADROOM: usize = 512;
const DEFAULT_SEND_TIMEOUT: Duration = Duration::from_secs(10);

mod demands;
mod publication;
#[cfg(test)]
pub(crate) use publication::PublishError;

pub(crate) fn database_capacity() -> NonZeroUsize {
    NonZeroUsize::new(
        CONTENT_EVENT_LIMIT
            + GROUP_RECORD_LIMIT
            + PROFILE_EVENT_LIMIT
            + REFERENCE_EVENT_LIMIT
            + DATABASE_HEADROOM,
    )
    .expect("Nostr database capacity is non-zero")
}

#[derive(Clone)]
struct Demand {
    id: SubscriptionId,
    filters: Vec<Filter>,
}

/// One complete raw level for the currently active product demands.
pub(crate) struct RawSnapshot {
    pub(crate) events: Vec<Event>,
    pub(crate) earliest_expiration: Option<Timestamp>,
}

/// Thin daemon-owned adapter around one `nostr-sdk` client.
pub(crate) struct NostrRuntime {
    client: Client,
    relay: RelayUrl,
    demands: Mutex<BTreeMap<String, Demand>>,
    completed: Mutex<BTreeSet<SubscriptionId>>,
    /// Source of per-REQ subscription ids. A replaced demand never reuses the
    /// id of the subscription it replaces; see `replace_demand`.
    subscription_sequence: std::sync::atomic::AtomicU64,
    /// Subscription ids sent but not yet installed as a demand. Their EOSE may
    /// arrive before the install, so completion pruning must keep them.
    opening: Mutex<BTreeSet<SubscriptionId>>,
    reference_read: tokio::sync::Mutex<()>,
    changed: watch::Sender<u64>,
    send_timeout: Duration,
}

impl NostrRuntime {
    pub(crate) fn new(relay: &str) -> Result<Self> {
        Self::with_send_timeout(relay, DEFAULT_SEND_TIMEOUT)
    }

    pub(crate) fn with_send_timeout(relay: &str, send_timeout: Duration) -> Result<Self> {
        let relay = RelayUrl::parse(relay).with_context(|| format!("invalid relay {relay}"))?;
        let database = MemoryDatabase::bounded(database_capacity());
        let client = Client::builder()
            .database(database)
            .max_relays(NonZeroUsize::new(1))
            .verify_subscriptions(true)
            .build();
        let (changed, _) = watch::channel(0);
        Ok(Self {
            client,
            relay,
            demands: Mutex::new(BTreeMap::new()),
            completed: Mutex::new(BTreeSet::new()),
            subscription_sequence: std::sync::atomic::AtomicU64::new(0),
            opening: Mutex::new(BTreeSet::new()),
            reference_read: tokio::sync::Mutex::new(()),
            changed,
            send_timeout,
        })
    }

    pub(crate) async fn start(self: &Arc<Self>) -> Result<()> {
        let mut notifications = self.client.notifications();
        let runtime = Arc::clone(self);
        tokio::spawn(async move {
            while let Some(notification) = notifications.next().await {
                let relevant = match notification {
                    ClientNotification::Event { .. } => true,
                    ClientNotification::Message { message, .. } => match *message {
                        RelayMessage::EndOfStoredEvents(id) => {
                            runtime
                                .completed
                                .lock()
                                .expect("Nostr completion state poisoned")
                                .insert(id.into_owned());
                            true
                        }
                        RelayMessage::Closed {
                            subscription_id, ..
                        } => {
                            runtime
                                .completed
                                .lock()
                                .expect("Nostr completion state poisoned")
                                .remove(subscription_id.as_ref());
                            true
                        }
                        _ => false,
                    },
                    ClientNotification::Shutdown => false,
                };
                if relevant {
                    runtime.wake();
                }
            }
        });
        self.client
            .add_relay(self.relay.clone())
            .await
            .context("adding configured relay")?;
        self.client
            .connect_relay(self.relay.clone())
            .await
            .context("starting configured relay connection")?;
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        self.client.shutdown().await;
    }

    #[cfg(test)]
    pub(crate) fn is_shutdown(&self) -> bool {
        self.client.is_shutdown()
    }

    #[cfg(test)]
    async fn is_connected(&self) -> bool {
        self.client
            .relay(self.relay.clone())
            .await
            .ok()
            .flatten()
            .is_some_and(|relay| relay.status().is_connected())
    }

    pub(crate) fn subscribe_changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    #[cfg(feature = "stress-harness")]
    pub(crate) fn active_demand_count(&self) -> usize {
        self.demands.lock().expect("Nostr demands poisoned").len()
    }

    #[cfg(feature = "stress-harness")]
    pub(crate) fn profile_author_count(&self) -> usize {
        self.demands
            .lock()
            .expect("Nostr demands poisoned")
            .get("mosaico-profiles")
            .and_then(|demand| demand.filters.first())
            .map(|filter| filter.authors.as_ref().map_or(0, |authors| authors.len()))
            .unwrap_or(0)
    }

    fn wake(&self) {
        self.changed
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

#[cfg(test)]
#[path = "nostr_runtime/tests.rs"]
mod tests;
