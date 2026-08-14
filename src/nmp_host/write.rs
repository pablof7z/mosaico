//! NIP-29 writes and account lifecycle behind the NMP facade.
//!
//! **NMP signs, NMP stores, and Mosaico reads its own write back.** A group
//! write leaves through `nip29::Group::publish` or `nip29::Groups::publish`:
//! NMP appends the `h` rows, stamps, signs, takes durable custody and injects
//! the accepted row into the very subscription Mosaico already holds for that
//! group (NMP #1182). There is no app-side signing step, no app-side event id
//! derivation, and no app-local copy of the event.
//!
//! Ordinary fabric writes are optimistic: acceptance transfers durable
//! delivery ownership to NMP and returns immediately. Group-management
//! commands are different because their caller needs the relay's answer.
//! Those commands consume NMP's terminal [`ReceiptResult`](nmp::ReceiptResult)
//! instead of polling Mosaico's projected roster or reducing receipt frames.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nmp::{ReceiptStream, SignEventRequest};
use nmp_grammar::{Identity, WriteIntent, WritePayload, WriteRouting};
use nostr::{Event, EventBuilder, EventId, Keys, PublicKey, UnsignedEvent};

use super::scrub::scrub_unsigned;
use super::NmpHost;

mod compose;
mod group_management;
use compose::draft_of;

#[cfg(test)]
impl NmpHost {
    pub(crate) fn publish_queue_entry_ids(&self) -> Result<Vec<String>, String> {
        self.engine
            .publish_queue()
            .map(|entries| {
                entries
                    .into_iter()
                    .map(|entry| entry.event_id.to_hex())
                    .collect()
            })
            .map_err(|error| error.to_string())
    }
}

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
    ///
    /// This signs WITHOUT publishing, which is the only thing it is for. A
    /// group write never comes through here: NMP signs those inside its own
    /// publish door.
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

    /// Publish a draft into ONE NIP-29 group, as `keys`.
    ///
    /// The whole write: NMP appends the `h` row before the stamp/sign step,
    /// signs, takes custody and routes to every configured host. Mosaico
    /// supplies a draft and an author and holds nothing else.
    pub(crate) fn publish_group(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.publish_groups(std::iter::once(group.to_string()), builder, keys)
    }

    /// Publish a draft into EVERY named group at once, as `keys`.
    ///
    /// One event, one replaceable coordinate, one `h` row per group — the
    /// kind:30315 session status is the case in hand. One group is not a
    /// special path that happens to agree with this one; it is literally this
    /// one, which is why [`Self::publish_group`] delegates here.
    pub(crate) fn publish_groups(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        Ok(self.publish_groups_receipt(groups, builder, keys)?.event_id)
    }

    pub(super) fn publish_groups_receipt(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<ReceiptStream> {
        self.ensure_identity(keys)?;
        let author = keys.public_key();
        let mut unsigned = builder.build(author);
        scrub_unsigned(&mut unsigned);
        let groups: BTreeSet<String> = groups.into_iter().collect();
        self.publish_through_group_door(&groups, author, draft_of(unsigned))
    }

    /// Publish into the NIP-29 group door and return the event id acceptance
    /// froze.
    ///
    /// The id comes off the `ReceiptStream` the call already returns, because
    /// the transaction that issued the receipt decided it: it IS the write's
    /// identity from acceptance onward, and it is the post-restamp value in
    /// every case including a replaceable edit (NMP #1315). Mosaico neither
    /// derives it — reimplementing NIP-01's hashing rule was a second
    /// authority on the same fact — nor reads it back out of the publish
    /// queue, which materializes every retained receipt to answer a question
    /// about one write.
    fn publish_through_group_door(
        &self,
        groups: &BTreeSet<String>,
        author: PublicKey,
        builder: nmp::EventBuilder,
    ) -> Result<ReceiptStream> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let hosts = nmp::nip29::on(self.relays.iter().cloned())
            .map_err(|error| anyhow::anyhow!("no configured NIP-29 group host: {error:?}"))?;
        let stream = hosts
            .groups(groups.iter().cloned())
            .map_err(|error| anyhow::anyhow!("naming the groups of a write: {error}"))?
            .publish(&self.engine, author, builder)
            .map_err(|error| anyhow::anyhow!("publishing a NIP-29 group write: {error}"))?;
        Ok(stream)
    }

    /// Publish exact signed bytes to an exact relay set.
    ///
    /// NOT a group door and never used for one: NIP-29 composition, signing
    /// and routing all belong to `nip29::Groups::publish`. This is the plain
    /// NIP-01 write — a kind:0 profile going to the indexer, and the schema-7
    /// migration journal replaying bytes an older Mosaico signed before NMP
    /// existed. Neither can be re-composed, because re-composing would change
    /// the id.
    pub(crate) fn publish_signed_to(
        &self,
        relays: Vec<nmp::RelayUrl>,
        event: &Event,
    ) -> Result<ReceiptStream> {
        if relays.is_empty() {
            anyhow::bail!("cannot publish {} without a configured relay", event.id);
        }
        self.engine
            .publish(WriteIntent {
                payload: WritePayload::Signed(event.clone()),
                routing: WriteRouting::Explicit(relays),
                identity: Identity::Explicit(event.pubkey),
                correlation: None,
            })
            .context("submitting a signed NMP write")
    }

    /// Publish a kind:0 copy to every configured app/indexer relay.
    pub(crate) fn enqueue_profile_event(&self, event: &Event) -> Result<EventId> {
        if event.kind.as_u16() != 0 {
            anyhow::bail!(
                "profile enqueue requires kind:0, got {}",
                event.kind.as_u16()
            );
        }
        self.publish_signed_to(self.profile_relays.iter().cloned().collect(), event)?;
        Ok(event.id)
    }

    /// Every configured NIP-29 group host, for a write that names its own
    /// groups in bytes Mosaico may not recompose.
    pub(crate) fn group_hosts(&self) -> Vec<nmp::RelayUrl> {
        self.relays.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests;
