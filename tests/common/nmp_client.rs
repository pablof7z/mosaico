//! Test relay client backed by fava, replacing the old NMP engine facade.

#![allow(dead_code)]

use anyhow::{Context, Result};
use fava::RelayUrl;
use nostr::event::{Event, EventBuilder, EventId, FinalizeEvent as _};
use nostr::filter::Filter;
use nostr::key::Keys;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use fava::FavaBuilder;
use fava_delivery_standard::StandardDeliveryPolicy;
use fava_event_cache_memory::MemoryEventCache;
use fava_publisher_nip01::Nip01Publisher;
use fava_query_standard::StandardQueryEvaluator;
use fava_router_app_relays::AppRelayRouter;
use fava_signer_local::LocalSigner;
use fava_subscriptions_no_grouping::planner;
use fava_transport_websocket::WebSocketTransport;
use fava_write_store_memory::MemoryWriteStore;

pub struct NmpRelayClient {
    fava: fava::Fava,
    relay: RelayUrl,
    keys: Keys,
}

#[derive(Debug)]
pub struct WriteAck {
    pub val: EventId,
    pub success: BTreeSet<String>,
    pub failed: BTreeMap<String, String>,
}

impl NmpRelayClient {
    pub async fn connect(keys: Keys, relay: &str) -> Result<Self> {
        let relay = RelayUrl::parse(relay).context("parse test relay URL")?;
        let fava = FavaBuilder::default()
            .event_cache(Arc::new(MemoryEventCache::default()))
            .query_evaluator(Arc::new(StandardQueryEvaluator))
            .subscription_planner(Arc::new(planner()))
            .transport(Arc::new(WebSocketTransport::new()))
            .router(Arc::new(AppRelayRouter::new("app", [relay.clone()])))
            .signer(Arc::new(LocalSigner::new(keys.clone())))
            .publisher(Arc::new(Nip01Publisher))
            .delivery_policy(Arc::new(StandardDeliveryPolicy::default()))
            .write_store(Arc::new(MemoryWriteStore::default()))
            .build()
            .context("build fava test client")?;
        Ok(Self { fava, relay, keys })
    }

    pub fn register_identity(&mut self, _keys: &Keys) -> Result<()> {
        // In fava the signer is per-client; multi-key publish uses pre-signed events.
        // This is a no-op for tests that send pre-signed events via send_event().
        Ok(())
    }

    pub async fn send_event_builder(&self, builder: EventBuilder) -> Result<WriteAck> {
        let event = builder.finalize(&self.keys).context("sign test event")?;
        self.send_event(&event).await
    }

    /// Publish an unsigned Fava builder through the client's registered signer.
    pub async fn send_fava_builder(&self, builder: fava::EventBuilder) -> Result<WriteAck> {
        let relay_url = self.relay.clone();
        let relay_str = relay_url.to_string();
        let write = self
            .fava
            .by(self.keys.public_key())
            .publish(builder)
            .context("submit Fava test write")?;
        let event_id = write
            .receipt()
            .context("read Fava test write receipt")?
            .current
            .id();
        match write.settled(fava::all_terminal()).await {
            Ok(_) => Ok(WriteAck {
                val: event_id,
                success: BTreeSet::from([relay_str]),
                failed: BTreeMap::new(),
            }),
            Err(fava::PublishError::NotReached { receipt }) => {
                // Route settled but not every destination accepted — report per-relay.
                let mut success = BTreeSet::new();
                let mut failed = BTreeMap::new();
                for (session, outcome) in receipt.destinations() {
                    let dest = session.relay.to_string();
                    use fava::RelayDeliveryOutcome;
                    match outcome {
                        RelayDeliveryOutcome::Acknowledged { .. } => {
                            success.insert(dest);
                        }
                        RelayDeliveryOutcome::Rejected { message } => {
                            failed.insert(dest, message.clone());
                        }
                        _ => {
                            failed.insert(dest, format!("{outcome:?}"));
                        }
                    }
                }
                Ok(WriteAck {
                    val: event_id,
                    success,
                    failed,
                })
            }
            Err(error) => Err(anyhow::anyhow!("Fava test write failed: {error:#}")),
        }
    }

    pub async fn send_event(&self, event: &Event) -> Result<WriteAck> {
        let relay_url = self.relay.clone();
        let event_id = event.id;
        let relay_str = relay_url.to_string();
        let write = self
            .fava
            .to([relay_url])
            .context("build relay route")?
            .publish(event.clone())
            .context("submit fava test write")?;
        match write.settled(fava::all_terminal()).await {
            Ok(_) => Ok(WriteAck {
                val: event_id,
                success: BTreeSet::from([relay_str]),
                failed: BTreeMap::new(),
            }),
            Err(fava::PublishError::NotReached { receipt }) => {
                // Route settled but not every destination accepted — report per-relay.
                let mut success = BTreeSet::new();
                let mut failed = BTreeMap::new();
                for (session, outcome) in receipt.destinations() {
                    let dest = session.relay.to_string();
                    use fava::RelayDeliveryOutcome;
                    match outcome {
                        RelayDeliveryOutcome::Acknowledged { .. } => { success.insert(dest); }
                        RelayDeliveryOutcome::Rejected { message } => {
                            failed.insert(dest, message.clone());
                        }
                        _ => { failed.insert(dest, format!("{outcome:?}")); }
                    }
                }
                Ok(WriteAck { val: event_id, success, failed })
            }
            Err(e) => Err(anyhow::anyhow!("fava test write failed: {e:#}")),
        }
    }

    pub async fn fetch_events(&self, filter: Filter, timeout: Duration) -> Result<Vec<Event>> {
        use fava_query::Query;
        let relay = self.relay.clone();

        let mut query = Query::events()
            .only_from_relays([relay])
            .context("build pinned relay query")?;
        if let Some(kinds) = filter.kinds {
            query = query
                .kinds(kinds)
                .context("apply kinds to query")?;
        }
        if let Some(authors) = filter.authors {
            query = query
                .authors(authors)
                .context("apply authors to query")?;
        }
        if let Some(ids) = filter.ids {
            query = query.ids(ids).context("apply ids to query")?;
        }
        if let Some(limit) = filter.limit {
            query = query.limit(limit).context("apply limit to query")?;
        }

        let mut observation = self
            .fava
            .observe(query)
            .await
            .context("open fava test observation")?;

        let deadline = std::time::Instant::now() + timeout;
        let mut snapshot = observation.current();

        loop {
            // Acquisition is done when at least one relay has sent EOSE or
            // withdrawn (refused, unreachable, timed out, etc.).
            let settled = snapshot.evidence.relays.iter().any(|relay| {
                use fava_query::RelaySourceState;
                matches!(
                    relay.state,
                    RelaySourceState::StoredEventsComplete { .. }
                        | RelaySourceState::Refused { .. }
                        | RelaySourceState::TimedOut { .. }
                        | RelaySourceState::Unreachable { .. }
                        | RelaySourceState::Withdrawn { .. }
                )
            });
            if settled {
                break;
            }
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            match tokio::time::timeout(remaining, observation.changed()).await {
                Ok(Ok(s)) => snapshot = s,
                _ => break,
            }
        }
        observation.close();

        let events = snapshot
            .events
            .iter()
            .filter_map(|rec| {
                if let fava::EventValue::Signed(e) = rec.event() {
                    Some(e.clone())
                } else {
                    None
                }
            })
            .collect();
        Ok(events)
    }

    pub async fn disconnect(&self) {
        // Fava cleanup happens on drop; no explicit shutdown needed.
    }
}
