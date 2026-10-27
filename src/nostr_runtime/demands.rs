use super::*;
use nostr::prelude::{Kind, SingleLetterTag};

impl NostrRuntime {
    pub(crate) async fn replace_demand(
        self: &Arc<Self>,
        product_id: &str,
        filters: Vec<Filter>,
    ) -> Result<()> {
        if filters.is_empty() {
            return self.close_demand(product_id).await;
        }
        // Relays may handle one connection's messages concurrently (khatru
        // runs each frame in its own goroutine), so a CLOSE followed by a REQ
        // reusing the same subscription id can be applied in reverse and
        // silently drop the new live subscription. Every REQ therefore gets a
        // fresh id, and the replaced subscription is closed only after its
        // successor is open: a late CLOSE can only ever reach the old id.
        let sequence = self
            .subscription_sequence
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = SubscriptionId::new(format!("{product_id}:{sequence}"));
        self.opening
            .lock()
            .expect("opening subscriptions poisoned")
            .insert(id.clone());
        let opened = self
            .client
            .subscribe(nostr_sdk::client::ReqTarget::single(
                self.relay.clone(),
                filters.clone(),
            ))
            .with_id(id.clone())
            .await;
        let accepted = matches!(
            &opened,
            Ok(output) if output.failed.is_empty() && output.success.contains_key(&self.relay)
        );
        if !accepted {
            // Keep the prior subscription, which still serves its demand, and
            // drop whatever part of the failed replacement the SDK registered.
            self.opening
                .lock()
                .expect("opening subscriptions poisoned")
                .remove(&id);
            let _ = self.client.unsubscribe(&id).await;
            self.prune_completed();
            return match opened {
                Err(error) => {
                    Err(error).with_context(|| format!("opening Nostr demand {product_id}"))
                }
                Ok(_) => anyhow::bail!("configured relay did not accept Nostr demand {product_id}"),
            };
        }
        let prior = {
            let mut demands = self.demands.lock().expect("demands poisoned");
            self.opening
                .lock()
                .expect("opening subscriptions poisoned")
                .remove(&id);
            demands.insert(product_id.to_string(), Demand { id, filters })
        };
        if let Some(prior) = prior {
            // The replacement already serves the demand. A failed CLOSE of the
            // old id must not fail it: the old subscription is unregistered
            // locally either way, and the relay drops it with the connection.
            if let Err(error) = self.close_subscription(product_id, &prior.id).await {
                tracing::warn!(product_id, error = %format!("{error:#}"), "closing replaced Nostr subscription failed");
            }
        }
        self.wake();
        Ok(())
    }

    pub(crate) async fn apply(
        self: &Arc<Self>,
        effect: &crate::reconcile::SubEffect,
    ) -> Result<()> {
        use crate::reconcile::{SubEffect, SubscriptionSpec};
        match effect {
            SubEffect::Close { id } => self.close_demand(id).await,
            SubEffect::Open { id, query } | SubEffect::Replace { id, query } => {
                let filter = match query {
                    SubscriptionSpec::GroupContentsMulti { groups, kinds } => Filter::new()
                        .kinds(
                            kinds
                                .iter()
                                .copied()
                                .map(Kind::Custom)
                                .chain(std::iter::once(Kind::EventDeletion)),
                        )
                        .custom_tags(
                            SingleLetterTag::LOWERCASE_H,
                            groups.iter().map(String::as_str),
                        )
                        .limit(CONTENT_EVENT_LIMIT),
                    SubscriptionSpec::References { event_id, kinds } => Filter::new()
                        .kinds(kinds.iter().copied().map(Kind::Custom))
                        .custom_tag(SingleLetterTag::LOWERCASE_E, event_id)
                        .limit(REFERENCE_EVENT_LIMIT),
                };
                self.replace_demand(id, vec![filter]).await
            }
        }
    }

    pub(crate) async fn close_demand(&self, product_id: &str) -> Result<()> {
        let prior = self
            .demands
            .lock()
            .expect("demands poisoned")
            .remove(product_id);
        if let Some(prior) = prior {
            self.close_subscription(product_id, &prior.id).await?;
            self.wake();
        }
        Ok(())
    }

    async fn close_subscription(&self, product_id: &str, id: &SubscriptionId) -> Result<()> {
        let output = self.client.unsubscribe(id).await;
        // Fresh ids mean a late EOSE for a closed subscription would otherwise
        // stay in the completion set forever. Pruning here (not at EOSE time,
        // which can precede the demand install) bounds it to live and opening
        // subscriptions.
        self.prune_completed();
        let output = output.with_context(|| format!("closing Nostr demand {product_id}"))?;
        if !output.failed.is_empty() {
            anyhow::bail!("configured relay did not close Nostr demand {product_id}")
        }
        Ok(())
    }

    fn prune_completed(&self) {
        let demands = self.demands.lock().expect("demands poisoned");
        let opening = self.opening.lock().expect("opening subscriptions poisoned");
        self.completed
            .lock()
            .expect("completion state poisoned")
            .retain(|id| opening.contains(id) || demands.values().any(|demand| &demand.id == id));
    }

    pub(crate) async fn snapshot(&self) -> Result<RawSnapshot> {
        let filters = self
            .demands
            .lock()
            .expect("demands poisoned")
            .values()
            .flat_map(|demand| demand.filters.clone())
            .collect::<Vec<_>>();
        let now = Timestamp::now();
        let mut events = BTreeMap::new();
        for filter in filters {
            for event in self
                .client
                .database()
                .query(filter)
                .await
                .context("querying bounded Nostr database")?
            {
                if !event.is_expired_at(now) {
                    events.insert(event.id, event);
                }
            }
        }
        let events = events.into_values().collect::<Vec<_>>();
        let earliest_expiration = events
            .iter()
            .filter_map(|event| event.tags.expiration())
            .min();
        Ok(RawSnapshot {
            events,
            earliest_expiration,
        })
    }

    pub(crate) async fn wait_for_eose(&self, product_id: &str, timeout: Duration) -> Result<()> {
        let id = self
            .demands
            .lock()
            .expect("demands poisoned")
            .get(product_id)
            .map(|demand| demand.id.clone())
            .with_context(|| format!("Nostr demand {product_id} is not open"))?;
        let mut changes = self.subscribe_changes();
        tokio::time::timeout(timeout, async {
            loop {
                if self
                    .completed
                    .lock()
                    .expect("completion state poisoned")
                    .contains(&id)
                {
                    return Ok(());
                }
                changes
                    .changed()
                    .await
                    .context("Nostr runtime stopped before EOSE")?;
            }
        })
        .await
        .with_context(|| format!("timed out waiting for EOSE on {product_id}"))?
    }

    pub(crate) async fn wait_for_reference(
        self: &Arc<Self>,
        event_id: &str,
        kinds: impl IntoIterator<Item = u16>,
        timeout: Duration,
    ) -> Result<Vec<Event>> {
        const ID: &str = "mosaico-reference-read";
        let _serial = self.reference_read.lock().await;
        let kinds = kinds.into_iter().collect::<BTreeSet<_>>();
        let filter = Filter::new()
            .kinds(kinds.iter().copied().map(Kind::Custom))
            .custom_tag(SingleLetterTag::LOWERCASE_E, event_id)
            .limit(REFERENCE_EVENT_LIMIT);
        self.replace_demand(ID, vec![filter]).await?;
        let mut changes = self.subscribe_changes();
        let result = tokio::time::timeout(timeout, async {
            loop {
                let events = self
                    .snapshot()
                    .await?
                    .events
                    .into_iter()
                    .filter(|event| kinds.contains(&event.kind.as_u16()))
                    .filter(|event| {
                        event.tags.iter().any(|tag| {
                            let tag = tag.as_slice();
                            tag.first().map(String::as_str) == Some("e")
                                && tag.get(1).map(String::as_str) == Some(event_id)
                        })
                    })
                    .collect::<Vec<_>>();
                if !events.is_empty() {
                    return Ok(events);
                }
                changes.changed().await.context("Nostr runtime stopped")?;
            }
        })
        .await
        .context("timed out waiting for referenced relay event")?;
        let close = self.close_demand(ID).await;
        match (result, close) {
            (Ok(events), Ok(())) => Ok(events),
            (Err(error), _) | (_, Err(error)) => Err(error),
        }
    }
}
