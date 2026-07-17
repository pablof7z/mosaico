//! NMP-backed Nostr acquisition and durable publication.
//!
//! NMP owns relay planning, subscription lifecycle, canonical wire-event
//! deduplication, and acquisition evidence. Mosaico keeps its product read model:
//! delivered events are projected into `state.db` by the existing fabric
//! materializer. NMP also owns every durable write intent, route, receipt, and
//! bounded retry; the provider supplies product policy and exact host authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use nmp::{
    AccessContext, AccountRegistration, Binding, Demand, Engine, EngineConfig, IndexedTagName,
    LiveQuery, ObservationCancel, RelayUrl, SourceAuthority,
};
use nostr_sdk::prelude::PublicKey;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
mod scrub;
mod write;

const MATERIALIZATION_QUEUE_CAPACITY: usize = 2048;

pub(crate) struct NmpHost {
    engine: Engine,
    relays: BTreeSet<RelayUrl>,
    profile_relays: BTreeSet<RelayUrl>,
    accounts: Mutex<BTreeMap<PublicKey, AccountRegistration>>,
    signing: Mutex<()>,
    subscriptions: Mutex<BTreeMap<String, ObservationCancel>>,
    materialization_tx: Mutex<Option<mpsc::Sender<nostr_sdk::Event>>>,
    materialization_rx: Mutex<Option<mpsc::Receiver<nostr_sdk::Event>>>,
}

impl NmpHost {
    pub(crate) fn open(
        relays: &[String],
        indexer_relay: Option<&str>,
        store_path: Option<&Path>,
    ) -> Result<Self> {
        let parsed = relays
            .iter()
            .map(|relay| RelayUrl::parse(relay).with_context(|| format!("invalid relay {relay}")))
            .collect::<Result<BTreeSet<_>>>()?;
        let mut config = EngineConfig {
            store_path: store_path.map(|path| path.to_string_lossy().into_owned()),
            app_relays: relays.to_vec(),
            allowed_local_relay_hosts: local_relay_hosts(parsed.iter()),
            ..EngineConfig::default()
        };
        // A daemon can host many durable agent identities over its lifetime.
        // Keep the registry finite, but do not inherit NMP's small demo default.
        config.max_auth_capabilities = 4096;
        let mut profile_relays = parsed.clone();
        if let Some(indexer) = indexer_relay.filter(|relay| !relay.is_empty()) {
            let parsed_indexer = RelayUrl::parse(indexer)
                .with_context(|| format!("invalid indexer relay {indexer}"))?;
            config
                .allowed_local_relay_hosts
                .extend(local_relay_hosts([&parsed_indexer]));
            config.allowed_local_relay_hosts.sort();
            config.allowed_local_relay_hosts.dedup();
            config.indexer_relays.push(indexer.to_string());
            profile_relays.insert(parsed_indexer);
        }
        let engine = Engine::new(config).context("starting NMP engine")?;
        let (materialization_tx, materialization_rx) =
            mpsc::channel(MATERIALIZATION_QUEUE_CAPACITY);
        Ok(Self {
            engine,
            relays: parsed,
            profile_relays,
            accounts: Mutex::new(BTreeMap::new()),
            signing: Mutex::new(()),
            subscriptions: Mutex::new(BTreeMap::new()),
            materialization_tx: Mutex::new(Some(materialization_tx)),
            materialization_rx: Mutex::new(Some(materialization_rx)),
        })
    }

    /// Take the one lossless stream feeding Mosaico's canonical read-model
    /// materializer. A bounded channel deliberately backpressures observation
    /// drains instead of dropping canonical additions under a relay burst.
    pub(crate) fn take_materialization_events(&self) -> Result<mpsc::Receiver<nostr_sdk::Event>> {
        self.materialization_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .context("NMP materialization stream is already owned")
    }

    /// Open a caller-owned NMP observation. Dropping the returned value closes
    /// it, making this suitable for precise, short-lived correlation queries.
    pub(crate) fn observe(&self, query: &SubscriptionQuery) -> Result<nmp::Subscription> {
        self.engine
            .observe(live_query(&self.relays, query)?, None)
            .context("opening NMP observation")
    }

    pub(crate) fn shutdown(&self) {
        let subscriptions = std::mem::take(
            &mut *self
                .subscriptions
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for (_, cancel) in subscriptions {
            cancel.cancel();
        }
        self.materialization_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.engine.shutdown();
    }

    pub(crate) fn apply(&self, effect: &SubEffect) -> Result<()> {
        match effect {
            SubEffect::Open { id, query } | SubEffect::Replace { id, query } => {
                self.open_subscription(id, query)
            }
            SubEffect::Close { id } => {
                self.close_subscription(id);
                Ok(())
            }
        }
    }

    fn open_subscription(&self, id: &str, query: &SubscriptionQuery) -> Result<()> {
        let subscription = self
            .observe(query)
            .with_context(|| format!("opening NMP observation {id}"))?;
        let cancel = subscription.cancel_handle();
        let materialization = self
            .materialization_tx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
            .context("NMP host is shut down")?;
        std::thread::Builder::new()
            .name(format!("nmp-{id}"))
            .spawn(move || {
                while let Ok(frame) = subscription.recv() {
                    for event in frame.deltas.iter().filter_map(|delta| delta.event()) {
                        if materialization.blocking_send(event.clone()).is_err() {
                            return;
                        }
                    }
                }
            })
            .with_context(|| format!("starting NMP observation drain {id}"))?;
        let previous = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .insert(id.to_string(), cancel);
        if let Some(previous) = previous {
            previous.cancel();
        }
        Ok(())
    }

    fn close_subscription(&self, id: &str) {
        if let Some(cancel) = self
            .subscriptions
            .lock()
            .expect("NMP subscription mutex poisoned")
            .remove(id)
        {
            cancel.cancel();
        }
    }
}

impl Drop for NmpHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn live_query(relays: &BTreeSet<RelayUrl>, query: &SubscriptionQuery) -> Result<LiveQuery> {
    let mut filter = nmp::Filter {
        kinds: Some(query.kinds.clone()),
        ..nmp::Filter::default()
    };
    if let Some((name, value)) = &query.tag {
        let tag = IndexedTagName::new(*name)
            .with_context(|| format!("invalid indexed tag name {name}"))?;
        filter
            .tags
            .insert(tag, Binding::Literal(BTreeSet::from([value.to_string()])));
    }
    let demand = if relays.is_empty() {
        Demand::from_filter(filter)
    } else {
        Demand::new(
            filter,
            SourceAuthority::Pinned(relays.clone()),
            AccessContext::Public,
        )?
    };
    Ok(LiveQuery(demand))
}

fn local_relay_hosts<'a>(relays: impl IntoIterator<Item = &'a RelayUrl>) -> Vec<String> {
    relays
        .into_iter()
        .filter(|relay| !nmp::admits_network_relay_hint(relay))
        .filter_map(|relay| {
            url::Url::parse(relay.as_str())
                .ok()?
                .host_str()
                .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
        })
        // Onion routing is local in transport terms but not a local-network
        // SSRF opt-in. NMP handles it as a separate trust class.
        .filter(|host| !host.ends_with(".onion"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_is_public_and_pinned_to_the_configured_host() {
        let relay = RelayUrl::parse("wss://relay.example.com").unwrap();
        let relays = BTreeSet::from([relay.clone()]);
        let query = SubscriptionQuery {
            kinds: BTreeSet::from([9, 30315]),
            tag: Some(('h', "room".into())),
        };

        let live = live_query(&relays, &query).unwrap();
        assert_eq!(live.0.access, AccessContext::Public);
        assert_eq!(live.0.source, SourceAuthority::Pinned(relays));
        assert_eq!(live.0.selection.kinds, Some(query.kinds));
        let h = IndexedTagName::new('h').unwrap();
        assert_eq!(
            live.0.selection.tags.get(&h),
            Some(&Binding::Literal(BTreeSet::from(["room".to_string()])))
        );
    }

    #[test]
    fn configured_local_hosts_are_explicitly_allowed_but_onion_is_not() {
        let local = RelayUrl::parse("ws://127.0.0.1:7777").unwrap();
        let public = RelayUrl::parse("wss://relay.example.com").unwrap();
        let onion = RelayUrl::parse("ws://examplehiddenservice.onion").unwrap();

        assert_eq!(
            local_relay_hosts([&local, &public, &onion]),
            vec!["127.0.0.1"]
        );
    }

    #[test]
    fn canonical_materialization_stream_has_exactly_one_owner() {
        let host = NmpHost::open(&[], None, None).unwrap();
        let receiver = host.take_materialization_events().unwrap();
        assert!(host.take_materialization_events().is_err());
        drop(receiver);
    }
}
