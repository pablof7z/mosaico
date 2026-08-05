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
mod queue;
pub(crate) use background_receipts::BackgroundReceiptObserver;
pub(crate) use background_receipts::BackgroundWriteSnapshot;
#[cfg(test)]
use background_submit::collect_background_receivers;
use background_submit::BackgroundIntent;
use compose::{draft_of, frozen_id_of, unsigned_of};

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

    /// Sign a draft INTO `group`, so the group context tag is inside the bytes
    /// the signature covers.
    ///
    /// Mosaico signs its own chat, reaction and status bytes because it seeds
    /// them into the local read model before any relay has seen them, and an
    /// event id only exists once the body is frozen. `h` is still never
    /// Mosaico's to write: `nmp_nip29::contextualize` appends it, and refuses
    /// a draft that already carries one.
    ///
    /// The group id is named twice on this path -- once here and once at the
    /// `Group` the intent is minted from -- because `nip29::Group` retains an
    /// id but exposes no contextualizer. Tracked upstream as
    /// pablof7z/nmp#1283; one `group.contextualize(builder)` collapses both.
    pub(crate) async fn sign_group_event(
        self: &std::sync::Arc<Self>,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<Event> {
        let draft = contextualized_draft(group, builder, keys.public_key())?;
        self.sign_unsigned(draft, keys).await
    }

    /// Enqueue a NIP-29 write NMP will sign, and return the id it freezes at
    /// acceptance.
    ///
    /// The id is computed here rather than awaited: a NIP-01 id never depends
    /// on `sig`, so the frozen body already determines it, and NMP freezes the
    /// same bytes the group door minted. Waiting for `SigningState::Signed`
    /// would be waiting for something already known.
    pub(crate) fn publish_group_builder(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.ensure_identity(keys)?;
        let author = keys.public_key();
        let mut unsigned = builder.build(author);
        scrub_unsigned(&mut unsigned);
        let kind = unsigned.kind.as_u16();
        let intent = self
            .group(group)?
            .intent(author, draft_of(unsigned))
            .map_err(|error| anyhow::anyhow!("minting a NIP-29 group write: {error}"))?;
        let event_id = frozen_id_of(&intent, author)?;
        self.enqueue_background_intents(
            event_id,
            group_operation(kind),
            vec![BackgroundIntent {
                target: group.to_string(),
                intent,
            }],
        )?;
        Ok(event_id)
    }

    /// The NIP-29 group door for `group`, over every configured host.
    ///
    /// One `Group` mints ONE intent routed to the whole scope, which is why
    /// nothing here fans out per relay any more. The per-relay facts a write
    /// produces arrive on that one receipt stream and are folded by
    /// [`background_receipts::worker`]'s lane facts, which were already
    /// written for exactly that shape.
    pub(crate) fn group(&self, group: &str) -> Result<nmp::nip29::Group> {
        nmp::nip29::group(self.relays.iter().cloned(), group).map_err(|error| {
            anyhow::anyhow!("cannot publish into NIP-29 group {group:?}: {error:?}")
        })
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
}

/// The exact bytes a group write is signed over: Mosaico's draft with NMP's
/// group context row already inside it.
///
/// Public to the crate because it is what makes an event "in a group", and a
/// test that composes a group event any other way is testing a shape the
/// product never publishes.
pub(crate) fn contextualized_draft(
    group: &str,
    builder: EventBuilder,
    author: PublicKey,
) -> Result<UnsignedEvent> {
    let mut unsigned = builder.build(author);
    scrub_unsigned(&mut unsigned);
    let contextualized = nmp_nip29::contextualize(group, draft_of(unsigned))
        .map_err(|error| anyhow::anyhow!("composing a NIP-29 group draft: {error}"))?;
    Ok(unsigned_of(contextualized, author))
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
