//! Durable NIP-29 write and account lifecycle behind the NMP facade.

use std::collections::BTreeSet;
use std::sync::mpsc::Receiver;

use anyhow::{Context, Result};
use nmp::{RelayUrl, SignEventRequest, WriteStatus};
use nmp_grammar::{Durability, HostAuthority, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventBuilder, EventId, Keys, PublicKey, Tag, UnsignedEvent};

use super::scrub::scrub_unsigned;
use super::NmpHost;

mod receipt;
use receipt::wait_for_write;
#[cfg(test)]
use receipt::wait_for_write_blocking;

impl NmpHost {
    /// Sign an exact event through NMP's account registry. The facade's
    /// sign-only operation currently selects the active account, so this narrow
    /// critical section prevents concurrent session identities from racing it.
    pub(crate) async fn sign_event(
        self: &std::sync::Arc<Self>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<Event> {
        let host = std::sync::Arc::clone(self);
        let keys = keys.clone();
        tokio::task::spawn_blocking(move || {
            let _signing = host
                .signing
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            host.ensure_identity(&keys)?;
            let mut unsigned = builder.build(keys.public_key());
            scrub_unsigned(&mut unsigned);
            let previous = host
                .engine
                .active_account()
                .context("reading NMP account")?;
            host.engine
                .set_active_account(Some(keys.public_key()))
                .context("selecting NMP signing account")?;
            let result = (|| {
                host.engine
                    .sign_event(SignEventRequest {
                        created_at: unsigned.created_at,
                        kind: unsigned.kind,
                        tags: unsigned.tags.into_iter().collect(),
                        content: unsigned.content,
                    })
                    .context("starting NMP sign operation")?
                    .recv()
                    .context("signing event through NMP")
            })();
            let restored = host
                .engine
                .set_active_account(previous)
                .context("restoring NMP account");
            match (result, restored) {
                (Ok(event), Ok(())) => Ok(event),
                (Err(error), _) => Err(error),
                (Ok(_), Err(error)) => Err(error),
            }
        })
        .await
        .context("joining NMP signer")?
    }

    /// Durably enqueue a NIP-29 write and return once NMP has frozen and signed
    /// it. When `checked` is true, also wait for at least one relay ACK.
    pub(crate) async fn publish_group_builder(
        &self,
        builder: EventBuilder,
        keys: &Keys,
        checked: bool,
    ) -> Result<EventId> {
        self.ensure_identity(keys)?;
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        let author = keys.public_key();
        let receivers = self.publish_group_unsigned(unsigned, Some(author))?;
        wait_for_write(receivers, None, checked).await
    }

    /// Enqueue an already-signed group event. This is used when the provider
    /// needs the exact signed value for immediate local materialization.
    pub(crate) async fn publish_group_event(
        &self,
        event: &Event,
        checked: bool,
    ) -> Result<EventId> {
        let receivers = self.submit_signed_group(event)?;
        wait_for_write(receivers, Some(event.id), checked).await
    }

    /// Persist a signed group event behind NMP's crash-atomic acceptance door.
    /// Returns without waiting for signing, routing, relay I/O, or an ACK.
    pub(crate) fn enqueue_group_event(&self, event: &Event) -> Result<EventId> {
        drop(self.submit_signed_group(event)?);
        Ok(event.id)
    }

    /// Persist a kind:0 copy for every configured app/indexer relay. Profile
    /// Profile delivery has the same durable, independently-drained contract
    /// as every group write.
    pub(crate) fn enqueue_profile_event(&self, event: &Event) -> Result<EventId> {
        if event.kind.as_u16() != 0 {
            anyhow::bail!(
                "profile enqueue requires kind:0, got {}",
                event.kind.as_u16()
            );
        }
        let intents = self
            .profile_relays
            .iter()
            .cloned()
            .map(|relay| WriteIntent {
                payload: WritePayload::Signed(event.clone()),
                durability: Durability::Durable,
                routing: WriteRouting::PinnedHost(HostAuthority::from_selected_host(relay)),
                identity_override: Some(event.pubkey),
            })
            .collect::<Vec<_>>();
        drop(self.submit_intents(intents, "submitting profile NMP write")?);
        Ok(event.id)
    }

    fn submit_signed_group(&self, event: &Event) -> Result<Vec<Receiver<WriteStatus>>> {
        crate::relay_log::log_outgoing_event(event);
        let template = event_template(event)?;
        let intents = self
            .relays
            .iter()
            .cloned()
            .map(|relay| {
                let mut intent = group_intent(relay, template.clone())?;
                intent.payload = WritePayload::Signed(event.clone());
                intent.identity_override = Some(event.pubkey);
                Ok(intent)
            })
            .collect::<Result<Vec<_>>>()?;
        self.submit_intents(intents, "submitting signed NMP write")
    }

    /// The sole Mosaico -> NMP publication choke-point. `Engine::publish`
    /// synchronously confirms local durable acceptance and leaves all relay
    /// effects to NMP's independent retrying worker.
    fn submit_intents(
        &self,
        intents: Vec<WriteIntent>,
        context: &'static str,
    ) -> Result<Vec<Receiver<WriteStatus>>> {
        let receivers = intents
            .into_iter()
            .map(|intent| self.engine.publish(intent).context(context))
            .collect::<Result<Vec<_>>>()?;
        require_configured_host(&receivers)?;
        Ok(receivers)
    }

    fn publish_group_unsigned(
        &self,
        unsigned: UnsignedEvent,
        identity_override: Option<PublicKey>,
    ) -> Result<Vec<Receiver<WriteStatus>>> {
        let groups = group_values(unsigned.tags.iter());
        if groups.len() != 1 {
            anyhow::bail!(
                "unsigned NIP-29 writes require exactly one h tag; exact multi-group events must be pre-signed"
            );
        }
        let template = unsigned_template(&unsigned)?;
        let mut intents = Vec::with_capacity(self.relays.len());
        for relay in &self.relays {
            let mut intent = group_intent(relay.clone(), template.clone())?;
            intent.identity_override = identity_override;
            intents.push(intent);
        }
        self.submit_intents(intents, "submitting unsigned NMP write")
    }
}

#[derive(Clone)]
struct GroupTemplate {
    group: String,
    author: PublicKey,
    created_at: nostr::Timestamp,
    kind: u16,
    content: String,
    extra_tags: Vec<Vec<String>>,
}

fn unsigned_template(unsigned: &UnsignedEvent) -> Result<GroupTemplate> {
    group_template(
        unsigned.pubkey,
        unsigned.created_at,
        unsigned.kind.as_u16(),
        unsigned.content.clone(),
        unsigned.tags.iter().collect(),
    )
}

fn event_template(event: &Event) -> Result<GroupTemplate> {
    group_template(
        event.pubkey,
        event.created_at,
        event.kind.as_u16(),
        event.content.clone(),
        event.tags.iter().collect(),
    )
}

fn group_template(
    author: PublicKey,
    created_at: nostr::Timestamp,
    kind: u16,
    content: String,
    tags: Vec<&Tag>,
) -> Result<GroupTemplate> {
    let groups = group_values(tags.iter().copied());
    let group = groups
        .first()
        .cloned()
        .context("NIP-29 write has no h tag")?;
    let extra_tags = tags
        .into_iter()
        .filter(|tag| {
            !matches!(
                tag.as_slice().first().map(String::as_str),
                Some("h" | "previous")
            )
        })
        .map(|tag| tag.as_slice().to_vec())
        .collect();
    Ok(GroupTemplate {
        group,
        author,
        created_at,
        kind,
        content,
        extra_tags,
    })
}

fn group_values<'a>(tags: impl IntoIterator<Item = &'a Tag>) -> BTreeSet<String> {
    tags.into_iter()
        .filter_map(|tag| {
            let row = tag.as_slice();
            (row.first().map(String::as_str) == Some("h"))
                .then(|| row.get(1).cloned())
                .flatten()
        })
        .collect()
}

fn group_intent(relay: RelayUrl, template: GroupTemplate) -> Result<nmp::WriteIntent> {
    nmp_nip29::compose_group_send(
        relay,
        &template.group,
        template.author,
        template.created_at,
        template.kind,
        template.content,
        template.extra_tags,
        &nmp_nip29::GroupTimelineEvidence::none(),
    )
    .map_err(|error| anyhow::anyhow!("composing NMP group write: {error:?}"))
}

fn require_configured_host(receivers: &[Receiver<WriteStatus>]) -> Result<()> {
    if receivers.is_empty() {
        anyhow::bail!("cannot publish a NIP-29 event without a configured group host");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
