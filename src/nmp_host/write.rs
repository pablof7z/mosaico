//! Durable NIP-29 write and account lifecycle behind the NMP facade.
//!
//! **Publishing is optimistic.** `Engine::publish` returning `Ok` is NMP
//! taking custody: the write is durably recorded and whatever becomes of it is
//! recorded with it. Nothing here waits for a relay, because settlement is
//! something an app INSPECTS — through the background receipt observer's
//! evidence, and through NMP's own publish queue — never something it awaits.
//! The 12-second foreground deadline this module used to run was the mechanism
//! behind mosaico#745, where a terminal AUTH denial reached the operator as a
//! timeout.

use anyhow::{Context, Result};
use nmp::{FifoReceiver, SignEventRequest, WriteFact};
use nmp_grammar::{Identity, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventBuilder, EventId, Keys, PublicKey, UnsignedEvent};

use super::scrub::scrub_unsigned;
use super::NmpHost;

mod background_receipts;
mod background_submit;
mod compose;
pub(crate) use background_receipts::BackgroundReceiptObserver;
pub(crate) use background_receipts::BackgroundWriteSnapshot;
#[cfg(test)]
use background_submit::collect_background_receivers;
use background_submit::BackgroundIntent;
use compose::{
    contextualized_builder, event_template, frozen_event_id, group_intent, group_values,
    unsigned_template,
};
#[cfg(test)]
use compose::{group_template, GroupTemplate};

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

    /// Enqueue a NIP-29 write NMP will sign, and return the id it freezes at
    /// acceptance.
    ///
    /// The id is computed here rather than awaited: a NIP-01 id never depends
    /// on `sig`, so the frozen body already determines it, and NMP freezes the
    /// same bytes this composes. Waiting for `SigningState::Signed` would be
    /// waiting for something already known.
    pub(crate) fn publish_group_builder(
        &self,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.ensure_identity(keys)?;
        let mut unsigned = builder.build(keys.public_key());
        scrub_unsigned(&mut unsigned);
        let author = keys.public_key();
        let composed = self.unsigned_group_intents(&unsigned, author)?;
        self.enqueue_background_intents(
            composed.event_id,
            group_operation(unsigned.kind.as_u16()),
            composed.intents,
        )?;
        Ok(composed.event_id)
    }

    fn publish_intent(
        &self,
        intent: WriteIntent,
        context: &'static str,
    ) -> Result<FifoReceiver<WriteFact>> {
        #[cfg(test)]
        if let Some(result) = self.test_io.take_write() {
            return result.context(context);
        }
        self.engine.publish(intent).context(context)
    }

    fn unsigned_group_intents(
        &self,
        unsigned: &UnsignedEvent,
        author: PublicKey,
    ) -> Result<ComposedWrite> {
        let groups = group_values(unsigned.tags.iter());
        if groups.len() != 1 {
            anyhow::bail!(
                "unsigned NIP-29 writes require exactly one h tag; exact multi-group events must be pre-signed"
            );
        }
        let composed = contextualized_builder(unsigned_template(unsigned)?)?;
        let event_id = frozen_event_id(&composed, author)?;
        let intents = self
            .relays
            .iter()
            .enumerate()
            .map(|(index, relay)| BackgroundIntent {
                target: format!("{index}:{relay}"),
                intent: WriteIntent {
                    payload: WritePayload::Event(composed.clone()),
                    routing: WriteRouting::Explicit(vec![relay.clone()]),
                    identity: identity_of(Some(author)),
                    correlation: None,
                },
            })
            .collect();
        Ok(ComposedWrite { event_id, intents })
    }
}

/// A composed group write: the id NMP will freeze, and one intent per host.
struct ComposedWrite {
    event_id: EventId,
    intents: Vec<BackgroundIntent>,
}

/// The operation label correlated evidence is filed under.
pub(super) fn group_operation(kind: u16) -> &'static str {
    match kind {
        crate::fabric::nip29::wire::KIND_STATUS => "status",
        7 => "reaction",
        _ => "group_event",
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

#[cfg(test)]
mod tests;
