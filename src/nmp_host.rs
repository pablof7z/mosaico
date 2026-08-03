//! NMP-backed Nostr acquisition and durable publication.
//!
//! NMP owns relay planning, subscription lifecycle, canonical wire-event
//! deduplication, and acquisition evidence. Mosaico keeps its product read model:
//! delivered events are projected into `state.db` by the existing fabric
//! materializer. NMP also owns every durable write intent, route, receipt, and
//! bounded retry. Shared reads are public because Mosaico-created groups are
//! deliberately public; writes authenticate as the exact event author. The
//! provider supplies product policy and exact host authority.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use nmp::{
    AccessContext, Binding, CacheMode, Demand, Engine, EngineConfig, IndexedTagName, LiveQuery,
    ObservationCancel, RelayUrl, SourceAuthority,
};
use nostr::Keys;
use tokio::sync::mpsc;

use crate::reconcile::{SubEffect, SubscriptionQuery};
mod auth;
pub(crate) mod read;
mod scrub;
#[cfg(test)]
mod test_io;
mod write;

use auth::IdentityRegistration;
use write::BackgroundReceiptObserver;

const MATERIALIZATION_QUEUE_CAPACITY: usize = 2048;
const MAX_LOCAL_IDENTITIES: usize = 4096;

pub(crate) struct NmpHost {
    engine: Engine,
    relays: BTreeSet<RelayUrl>,
    profile_relays: BTreeSet<RelayUrl>,
    identities: Mutex<BTreeMap<nostr::PublicKey, IdentityRegistration>>,
    signing: Mutex<()>,
    subscriptions: Mutex<BTreeMap<String, ObservationCancel>>,
    materialization_tx: Mutex<Option<mpsc::Sender<nostr::Event>>>,
    materialization_rx: Mutex<Option<mpsc::Receiver<nostr::Event>>>,
    background_receipts: BackgroundReceiptObserver,
    #[cfg(test)]
    test_io: test_io::TestIo,
}

impl NmpHost {
    pub(crate) fn open(
        relays: &[String],
        indexer_relay: Option<&str>,
        store_path: Option<&Path>,
        backend_keys: &Keys,
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
        // Each identity consumes one signer registration and one AUTH-policy
        // registration from NMP's shared capability ceiling.
        config.max_auth_capabilities = MAX_LOCAL_IDENTITIES * 2;
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
        let background_receipts = BackgroundReceiptObserver::start()?;
        let host = Self {
            engine,
            relays: parsed,
            profile_relays,
            identities: Mutex::new(BTreeMap::new()),
            signing: Mutex::new(()),
            subscriptions: Mutex::new(BTreeMap::new()),
            materialization_tx: Mutex::new(Some(materialization_tx)),
            materialization_rx: Mutex::new(Some(materialization_rx)),
            background_receipts,
            #[cfg(test)]
            test_io: test_io::TestIo::default(),
        };
        host.ensure_identity(backend_keys)
            .context("registering backend NIP-42 identity")?;
        Ok(host)
    }

    /// Take the one lossless stream feeding Mosaico's canonical read-model
    /// materializer. A bounded channel deliberately backpressures observation
    /// drains instead of dropping canonical additions under a relay burst.
    pub(crate) fn take_materialization_events(&self) -> Result<mpsc::Receiver<nostr::Event>> {
        self.materialization_rx
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
            .context("NMP materialization stream is already owned")
    }

    /// Open a caller-owned NMP observation. Dropping the returned value closes
    /// it, making this suitable for precise, short-lived correlation queries.
    pub(crate) fn observe(&self, query: &SubscriptionQuery) -> Result<nmp::Subscription> {
        self.observe_with_access(query, AccessContext::Public)
    }

    fn observe_with_access(
        &self,
        query: &SubscriptionQuery,
        access: AccessContext,
    ) -> Result<nmp::Subscription> {
        self.engine
            .observe(self.live_query(query, access)?, None)
            .context("opening NMP observation")
    }

    /// The relays a NIP-29 group lives on, as NMP's own scope value. Every
    /// NIP-29 read is minted from here so the host set is named once and the
    /// per-host branch structure is NMP's to build, not Mosaico's to imitate.
    pub(crate) fn nip29_scope(&self) -> Result<nmp::nip29::RelayScope> {
        nmp::nip29::on(self.relays.iter().cloned())
            .map_err(|error| anyhow::anyhow!("NIP-29 relay scope: {error}"))
    }

    /// One group's read declaration for an app-supplied selection: one
    /// complete branch per host, each `Pinned` to that host alone and
    /// `CacheMode::Strict`, scoped by `#h`. NMP refuses a selection that
    /// already constrains `#h`, which is why the group id is a parameter here
    /// and never a tag the caller writes.
    pub(crate) fn group_contents_query(
        &self,
        group: &str,
        selection: nmp::Filter,
    ) -> Result<LiveQuery> {
        self.nip29_scope()?
            .group(group)
            .read(selection)
            .map_err(|error| anyhow::anyhow!("NIP-29 group read for {group:?}: {error}"))
    }

    /// The relay-signed records describing ONE group — kinds 39000/39001/39002
    /// joined on `d`. One complete branch per host, `Pinned` and `Strict` at
    /// every nesting level, because these three kinds are signed by the RELAY
    /// and a row relay B served is no evidence about relay A's group.
    pub(crate) fn group_records_query(&self, group: &str) -> Result<LiveQuery> {
        let predicate = Binding::Literal(BTreeSet::from([group.to_string()]));
        let branches = self
            .relays
            .iter()
            .map(|host| nmp_nip29::groups_where_at(host, predicate.clone()))
            .collect::<Vec<_>>();
        union_branches(branches)
    }

    /// Every group these hosts describe (kind:39000, unkeyed).
    ///
    /// NMP has no unpredicated group-listing constructor — `groups_where_at`
    /// requires a `d` predicate — so the branch is assembled here from NMP's
    /// own vocabulary rather than borrowed. It still stamps both axes per
    /// host, which is the property that matters. See the report accompanying
    /// mosaico#741: an `all_groups_at(host)` door would remove this.
    pub(crate) fn all_group_metadata_query(&self) -> Result<LiveQuery> {
        let selection = nmp::Filter {
            kinds: Some(BTreeSet::from([nmp_nip29::GROUP_METADATA_KIND])),
            ..nmp::Filter::default()
        };
        let branches = self
            .relays
            .iter()
            .map(|host| {
                let mut demand = Demand::new(
                    selection.clone(),
                    SourceAuthority::Pinned(BTreeSet::from([host.clone()])),
                    AccessContext::Public,
                )?;
                demand.cache = CacheMode::Strict;
                Ok(demand)
            })
            .collect::<Result<Vec<_>, nmp::DemandError>>()
            .map_err(|error| anyhow::anyhow!("group metadata listing: {error}"))?;
        union_branches(branches)
    }

    /// A read NMP's NIP-29 vocabulary does not mint, pinned to `relays` with
    /// an EXPLICIT cache mode.
    ///
    /// `cache` is a parameter and never a default because the default is
    /// `Agnostic` — "serve every matching cached row regardless of
    /// provenance" — and inheriting it silently is precisely the defect
    /// mosaico#741 records. The two rules Mosaico applies:
    ///
    /// * Pinned to the GROUP hosts → `Strict`. Those hosts are asked because
    ///   they are the authority for the answer; a row a different relay
    ///   served is not evidence about them. Mosaico's own not-yet-carried
    ///   writes stay visible regardless — NMP decides that by ours-versus-
    ///   foreign, not by carried-versus-uncarried.
    /// * Pinned to the PROFILE hosts → `Agnostic`. kind:0 is
    ///   self-authenticating, the answer does not depend on who served it,
    ///   and the indexer is in that set precisely so it can answer for
    ///   relays outside the app's own.
    fn host_pinned_query(
        &self,
        relays: &BTreeSet<RelayUrl>,
        filter: nmp::Filter,
        access: AccessContext,
        cache: CacheMode,
    ) -> Result<LiveQuery> {
        let demand = if relays.is_empty() {
            Demand::from_filter(filter)
        } else {
            let mut demand = Demand::new(filter, SourceAuthority::Pinned(relays.clone()), access)?;
            demand.cache = cache;
            demand
        };
        Ok(LiveQuery::single(demand))
    }

    fn live_query(&self, query: &SubscriptionQuery, access: AccessContext) -> Result<LiveQuery> {
        match query {
            SubscriptionQuery::AllGroupMetadata => self.all_group_metadata_query(),
            SubscriptionQuery::GroupRecords { group } => self.group_records_query(group),
            SubscriptionQuery::GroupContents { group, kinds } => {
                self.group_contents_query(group, kinds_filter(kinds))
            }
            SubscriptionQuery::Kinds { kinds } => self.host_pinned_query(
                &self.relays,
                kinds_filter(kinds),
                access,
                CacheMode::Strict,
            ),
            SubscriptionQuery::Mentions { pubkey, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter.tags.insert(
                    indexed_tag('p')?,
                    Binding::Literal(BTreeSet::from([pubkey.clone()])),
                );
                self.host_pinned_query(&self.relays, filter, access, CacheMode::Strict)
            }
            SubscriptionQuery::References { event_id, kinds } => {
                let mut filter = kinds_filter(kinds);
                filter.tags.insert(
                    indexed_tag('e')?,
                    Binding::Literal(BTreeSet::from([event_id.clone()])),
                );
                self.host_pinned_query(&self.relays, filter, access, CacheMode::Strict)
            }
            SubscriptionQuery::Profile { pubkey } => {
                let mut filter = kinds_filter(&BTreeSet::from([0u16]));
                filter.authors = Some(Binding::Literal(BTreeSet::from([pubkey.clone()])));
                self.host_pinned_query(&self.profile_relays, filter, access, CacheMode::Agnostic)
            }
        }
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
        self.background_receipts.begin_shutdown();
        self.engine.shutdown();
        self.background_receipts.shutdown();
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

fn kinds_filter(kinds: &BTreeSet<u16>) -> nmp::Filter {
    nmp::Filter {
        kinds: (!kinds.is_empty()).then(|| kinds.clone()),
        ..nmp::Filter::default()
    }
}

fn indexed_tag(name: char) -> Result<IndexedTagName> {
    IndexedTagName::new(name).with_context(|| format!("invalid indexed tag name {name}"))
}

/// One live query out of one complete branch per host, exactly as NMP's own
/// NIP-29 read door folds them (`nmp::nip29::read::one_live_query`).
fn union_branches(branches: Vec<Demand>) -> Result<LiveQuery> {
    let mut branches = branches;
    match branches.len() {
        0 => anyhow::bail!("no configured group host to read from"),
        1 => Ok(LiveQuery::single(
            branches.pop().expect("exactly one branch"),
        )),
        _ => LiveQuery::union(branches.into_iter().map(LiveQuery::single), None)
            .map_err(|error| anyhow::anyhow!("composing per-host read branches: {error}")),
    }
}

fn local_relay_hosts<'a>(relays: impl IntoIterator<Item = &'a RelayUrl>) -> Vec<String> {
    relays
        .into_iter()
        .filter_map(nmp_grammar::relay::relay_host_key)
        .filter(|host| {
            nmp_network_policy::classify_bare_host(host) == nmp_network_policy::HostClass::Local
        })
        // Onion routing is local in transport terms but not a local-network
        // SSRF opt-in. NMP handles it as a separate trust class.
        .filter(|host| !host.ends_with(".onion"))
        .collect()
}

#[cfg(test)]
mod tests;
