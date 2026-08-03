//! Durable NIP-29 write and account lifecycle behind the NMP facade.

use anyhow::{Context, Result};
use nmp::{FifoReceiver, RelayUrl, SignEventRequest, WriteStatus};
use nmp_grammar::{Durability, Identity, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventBuilder, EventId, Keys, PublicKey, Tag, UnsignedEvent};
use std::collections::BTreeSet;

use super::scrub::scrub_unsigned;
use super::NmpHost;

mod background_receipts;
mod background_submit;
mod receipt;
pub(crate) use background_receipts::BackgroundReceiptObserver;
pub(crate) use background_receipts::BackgroundWriteSnapshot;
#[cfg(test)]
use background_submit::{collect_background_receivers, BackgroundIntent};
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
        let unsigned = builder.build(keys.public_key());
        self.sign_unsigned(unsigned, keys).await
    }

    /// Sign a draft another layer already composed.
    ///
    /// A protocol module that owns an event's schema hands back an
    /// `UnsignedEvent` rather than a builder — `nmp-blossom`'s BUD-11
    /// authorization is the case in hand — because signing and publishing are
    /// orthogonal stages. Its `created_at` is part of the grant it composed,
    /// so it survives to the signature rather than being re-stamped here.
    pub(crate) async fn sign_unsigned(
        self: &std::sync::Arc<Self>,
        unsigned: UnsignedEvent,
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
            let mut unsigned = unsigned;
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

    /// The sole Mosaico -> NMP publication choke-point. `Engine::publish`
    /// synchronously confirms local durable acceptance and leaves all relay
    /// effects to NMP's independent retrying worker.
    fn submit_intents(
        &self,
        intents: Vec<WriteIntent>,
        context: &'static str,
    ) -> Result<Vec<FifoReceiver<WriteStatus>>> {
        let receivers = intents
            .into_iter()
            .map(|intent| self.publish_intent(intent, context))
            .collect::<Result<Vec<_>>>()?;
        require_configured_host(&receivers)?;
        Ok(receivers)
    }

    fn publish_intent(
        &self,
        intent: WriteIntent,
        context: &'static str,
    ) -> Result<FifoReceiver<WriteStatus>> {
        #[cfg(test)]
        if let Some(result) = self.test_io.take_write() {
            return result.context(context);
        }
        self.engine.publish(intent).context(context)
    }

    fn publish_group_unsigned(
        &self,
        unsigned: UnsignedEvent,
        identity_override: Option<PublicKey>,
    ) -> Result<Vec<FifoReceiver<WriteStatus>>> {
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
            intent.identity = identity_of(identity_override);
            intents.push(intent);
        }
        self.submit_intents(intents, "submitting unsigned NMP write")
    }
}

/// `None` restates NMP's own default: publish as whoever is active at
/// acceptance. `Some(pk)` names the key explicitly, which is what the signed
/// paths do — the author is already frozen in the bytes there.
pub(crate) fn identity_of(identity_override: Option<PublicKey>) -> Identity {
    match identity_override {
        Some(pubkey) => Identity::Explicit(pubkey),
        None => Identity::Active,
    }
}

#[derive(Clone)]
struct GroupTemplate {
    group: String,
    created_at: nostr::Timestamp,
    kind: u16,
    content: String,
    extra_tags: Vec<Vec<String>>,
}

fn unsigned_template(unsigned: &UnsignedEvent) -> Result<GroupTemplate> {
    group_template(
        unsigned.created_at,
        unsigned.kind.as_u16(),
        unsigned.content.clone(),
        unsigned.tags.iter().collect(),
    )
}

fn event_template(event: &Event) -> Result<GroupTemplate> {
    group_template(
        event.created_at,
        event.kind.as_u16(),
        event.content.clone(),
        event.tags.iter().collect(),
    )
}

fn group_template(
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

/// Mint the group write from NMP's `#h` contextualizer, pinned to the one
/// host. `nmp_nip29::contextualize` refuses a caller-supplied `h` or
/// `previous` row, so `group_template` strips both before we get here.
///
/// Routing and identity are assembled HERE only because `nmp::nip29::Group`
/// exposes no mint-without-publish door: `Group::intent` and
/// `through_the_one_door` are private, and `publish`/`publish_signed` publish
/// immediately and return receipts. Mosaico mints intents and submits them
/// later through one choke-point, so it cannot use a publish-now door. See
/// NMP #1242; this is a visible gap, not a deliberate bypass of the group.
fn group_intent(relay: RelayUrl, template: GroupTemplate) -> Result<nmp::WriteIntent> {
    let tags = template
        .extra_tags
        .into_iter()
        .map(|row| {
            Tag::parse(row).map_err(|error| anyhow::anyhow!("invalid NIP-29 tag: {error:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    let builder = nmp::EventBuilder {
        kind: nostr::Kind::from(template.kind),
        tags,
        content: template.content,
        created_at: Some(template.created_at),
    };
    let contextualized = nmp_nip29::contextualize(&template.group, builder)
        .map_err(|error| anyhow::anyhow!("composing NMP group write: {error:?}"))?;
    Ok(WriteIntent {
        payload: WritePayload::Event(contextualized),
        durability: Durability::Durable,
        routing: WriteRouting::Explicit(vec![relay]),
        // A group write says nothing about WHO is publishing; callers that
        // know the author overwrite this with `Identity::Explicit`.
        identity: Identity::Active,
        correlation: None,
    })
}

fn require_configured_host(receivers: &[FifoReceiver<WriteStatus>]) -> Result<()> {
    if receivers.is_empty() {
        anyhow::bail!("cannot publish a NIP-29 event without a configured group host");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
