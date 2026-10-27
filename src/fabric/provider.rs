//! Product-owned NIP-29 policy over the daemon's thin Nostr runtime.

pub(crate) mod chat;
mod decoding;
mod group_management;
mod group_state;
mod group_topology;
mod membership_confirmation;
mod profiles;
mod publish_gate;
mod reactions;
mod readiness;

pub(crate) use publish_gate::{ConfirmedGroupScope, PublishScopeUnverified};

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use nostr::prelude::{EventBuilder, EventId, FinalizeEvent, Keys, PublicKey};

use crate::domain::DomainEvent;
use crate::fabric::nip29::readiness::ChannelReadiness;
use crate::fabric::nip29::view::Nip29ViewStore;
use crate::fabric::nip29::wire::Nip29WireCodec;
use crate::fabric::{NostrEventCodec, RawEnvelope};
use crate::nostr_runtime::NostrRuntime;
use crate::state::Store;

pub const FABRIC: &str = "nip29";

#[allow(dead_code)]
pub trait FabricProvider {
    fn name(&self) -> &'static str;
}

/// NIP-29 product semantics. Transport remains in [`NostrRuntime`].
pub struct Nip29Provider {
    pub wire: Nip29WireCodec,
    pub store: Arc<Mutex<Store>>,
    pub(crate) runtime: Arc<NostrRuntime>,
    pub(crate) view: Arc<Nip29ViewStore>,
    management_pubkey: PublicKey,
    group_read: tokio::sync::Mutex<()>,
    profile_read: tokio::sync::Mutex<()>,
    pub user_nsec: Option<String>,
    pub whitelisted_pubkeys: Vec<String>,
    pub readiness: Arc<ChannelReadiness>,
}

impl Nip29Provider {
    pub(crate) fn new(
        runtime: Arc<NostrRuntime>,
        view: Arc<Nip29ViewStore>,
        store: Arc<Mutex<Store>>,
        management_pubkey: PublicKey,
        user_nsec: Option<String>,
        whitelisted_pubkeys: Vec<String>,
    ) -> Self {
        Self {
            wire: Nip29WireCodec,
            store,
            runtime,
            view,
            management_pubkey,
            group_read: tokio::sync::Mutex::new(()),
            profile_read: tokio::sync::Mutex::new(()),
            user_nsec,
            whitelisted_pubkeys,
            readiness: Arc::new(ChannelReadiness::default()),
        }
    }

    pub fn name(&self) -> &'static str {
        FABRIC
    }

    pub fn encode(&self, event: &DomainEvent) -> Result<EventBuilder> {
        self.wire.encode(event)
    }

    pub fn decode(&self, envelope: &RawEnvelope) -> Option<DomainEvent> {
        self.wire.decode(envelope)
    }

    /// Wakes when the decoded relay-signed group state changes.
    pub(crate) fn subscribe_group_changes(&self) -> tokio::sync::watch::Receiver<u64> {
        self.view.subscribe_group_changes()
    }

    pub(crate) fn management_pubkey(&self) -> String {
        self.management_pubkey.to_hex()
    }

    /// Rebuild one complete product level after runtime wakes or NIP-40 expiry.
    pub(crate) fn spawn_view_rebuilder(self: &Arc<Self>) {
        let provider = Arc::clone(self);
        let mut changes = self.runtime.subscribe_changes();
        tokio::spawn(async move {
            loop {
                let expiration = match provider.runtime.snapshot().await {
                    Ok(snapshot) => {
                        let expiration = snapshot.earliest_expiration;
                        // Decoding the whole level is CPU work; keep it off
                        // the async workers. Awaiting keeps replacements ordered.
                        let view = Arc::clone(&provider.view);
                        if let Err(error) =
                            tokio::task::spawn_blocking(move || view.replace(snapshot.events)).await
                        {
                            tracing::error!(%error, "decoding complete NIP-29 view");
                        }
                        expiration
                    }
                    Err(error) => {
                        tracing::error!(%error, "rebuilding complete NIP-29 view");
                        None
                    }
                };
                let expiration_wait = async {
                    match expiration {
                        Some(at) => {
                            let seconds = at.as_secs().saturating_sub(crate::util::now_secs());
                            tokio::time::sleep(Duration::from_secs(seconds)).await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::select! {
                    result = changes.changed() => if result.is_err() { return; },
                    _ = expiration_wait => {}
                }
            }
        });
    }

    pub(crate) async fn publish_groups(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let groups = groups.into_iter().collect::<BTreeSet<_>>();
        if groups.is_empty() || groups.iter().any(String::is_empty) {
            anyhow::bail!("a NIP-29 publication requires a non-empty group id");
        }
        let refs = groups.iter().map(String::as_str).collect::<Vec<_>>();
        let event = crate::fabric::nip29::signed_into_groups(&refs, builder, keys)?;
        self.runtime
            .publish(&event)
            .await
            .map_err(anyhow::Error::from)
    }

    pub(crate) async fn publish_group(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.publish_groups([group.to_string()], builder, keys)
            .await
    }

    pub(crate) async fn publish_built(
        &self,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let event = builder.finalize(keys).context("signing Nostr event")?;
        self.runtime
            .publish(&event)
            .await
            .map_err(anyhow::Error::from)
    }

    pub async fn enqueue(&self, event: &DomainEvent, keys: &Keys) -> Result<EventId> {
        match event {
            DomainEvent::Status(status) => self.enqueue_status(status, keys, true, None).await,
            _ => {
                self.enqueue_with_confirmed_scope(event, keys, None, None)
                    .await
            }
        }
    }

    pub(crate) async fn enqueue_status_after_confirmed_scope(
        &self,
        status: &crate::domain::Status,
        keys: &Keys,
        scope: &ConfirmedGroupScope,
        require_member: bool,
    ) -> Result<EventId> {
        self.enqueue_status(status, keys, require_member, Some(scope))
            .await
    }

    pub(crate) async fn enqueue_status(
        &self,
        status: &crate::domain::Status,
        keys: &Keys,
        require_member: bool,
        confirmed_scope: Option<&ConfirmedGroupScope>,
    ) -> Result<EventId> {
        self.enqueue_with_confirmed_scope(
            &DomainEvent::Status(status.clone()),
            keys,
            confirmed_scope,
            Some(require_member),
        )
        .await
    }

    async fn enqueue_with_confirmed_scope(
        &self,
        event: &DomainEvent,
        keys: &Keys,
        confirmed_scope: Option<&ConfirmedGroupScope>,
        status_requires_membership: Option<bool>,
    ) -> Result<EventId> {
        if matches!(event, DomainEvent::Profile(_)) {
            return self.publish_built(self.wire.encode(event)?, keys).await;
        }
        let signer = keys.public_key().to_hex();
        match event {
            DomainEvent::Status(status) => {
                let require_member = status_requires_membership.unwrap_or(true);
                for channel in &status.channels {
                    if !confirmed_scope
                        .is_some_and(|scope| scope.permits(channel, &signer, require_member))
                    {
                        self.verify_publish_scope(channel, &signer, require_member)
                            .await?;
                    }
                }
            }
            DomainEvent::ChatMessage(message) => {
                self.verify_publish_scope(&message.channel, &signer, true)
                    .await?;
            }
            DomainEvent::Reaction(reaction) => {
                self.verify_publish_scope(&reaction.channel, &signer, true)
                    .await?;
            }
            DomainEvent::Profile(_) => unreachable!("profiles returned above"),
        }
        let builder = self.wire.encode(event)?;
        match event {
            DomainEvent::Status(status) => {
                self.publish_groups(status.channels.iter().cloned(), builder, keys)
                    .await
            }
            DomainEvent::ChatMessage(message) => {
                self.publish_group(&message.channel, builder, keys).await
            }
            DomainEvent::Reaction(reaction) => {
                self.publish_group(&reaction.channel, builder, keys).await
            }
            DomainEvent::Profile(_) => unreachable!("profiles returned above"),
        }
    }

    pub(in crate::fabric::provider) fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> R {
        let store = self.store.lock().expect("store mutex poisoned");
        f(&store)
    }
}
