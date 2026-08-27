//! NIP-29 writes and profile publication behind the fava facade.
//!
//! **Fava signs, fava stores, and Mosaico reads its own write back.** A group
//! write goes through `fava.to(relays).publish(unsigned)`. Fava handles signing
//! via the registered `LocalSigner`, takes durable custody, and routes to every
//! configured host.
//!
//! Sign-only operations (BUD-11) bypass fava's publish door and call
//! `keys.sign_event(unsigned)` directly via nostr 0.45.3's `FinalizeEvent`.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use fava::RelayUrl;
use nostr::event::{EventBuilder, FinalizeEvent, FinalizeUnsignedEvent};
use nostr::event::{Event, EventId, UnsignedEvent};
use nostr::key::{Keys, PublicKey};

use super::scrub::scrub_unsigned;
use super::NmpHost;

pub(crate) mod group_management;
pub(crate) use group_management::GroupUser;

impl NmpHost {
    /// Sign an event through the local keypair. Does NOT publish.
    ///
    /// This is the sign-only door for BUD-11 and other operations that need
    /// a signed event without publishing it.
    pub(crate) async fn sign_event(
        self: &std::sync::Arc<Self>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<Event> {
        let unsigned = builder.finalize_unsigned(keys.public_key());
        self.sign_unsigned(unsigned, keys).await
    }

    /// Sign a draft another layer already composed. Does NOT publish.
    pub(crate) async fn sign_unsigned(
        self: &std::sync::Arc<Self>,
        unsigned: UnsignedEvent,
        keys: &Keys,
    ) -> Result<Event> {
        let keys = keys.clone();
        let mut unsigned = unsigned;
        scrub_unsigned(&mut unsigned);
        tokio::task::spawn_blocking(move || {
            unsigned
                .finalize(&keys)
                .map_err(|e| anyhow::anyhow!("signing event: {e}"))
        })
        .await
        .context("joining sign task")?
    }

    /// Publish a draft into ONE NIP-29 group, as `keys`.
    pub(crate) fn publish_group(
        &self,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.publish_groups(std::iter::once(group.to_string()), builder, keys)
    }

    /// Publish a draft into EVERY named group at once, as `keys`.
    pub(crate) fn publish_groups(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        self.publish_groups_receipt(groups, builder, keys)
    }

    pub(super) fn publish_groups_receipt(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let (event_id, _write) = self.publish_groups_with_handle(groups, builder, keys)?;
        Ok(event_id)
    }

    fn publish_groups_with_handle(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<(EventId, fava::Write)> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let groups: BTreeSet<String> = groups.into_iter().collect();
        let mut unsigned = builder.finalize_unsigned(keys.public_key());
        // The group context row (`h` tag) belongs to the group mechanism, not
        // the caller. A draft that supplies its own `h` tag is refused rather
        // than silently overwritten.
        if unsigned.tags.iter().any(|t| {
            t.as_slice().first().map(|s| s.as_str()) == Some("h")
        }) {
            anyhow::bail!("the `h` tag belongs to the group, not the caller");
        }
        scrub_unsigned(&mut unsigned);
        // Append `h` tags for every group.
        for group in &groups {
            unsigned.tags.push(
                nostr::event::Tag::parse(["h", group]).map_err(|e| anyhow::anyhow!("h tag: {e}"))?,
            );
        }
        let event = unsigned
            .finalize(keys)
            .map_err(|e| anyhow::anyhow!("signing group event: {e}"))?;
        let event_id = event.id;
        let write = self.fava
            .to(self.relays.iter().cloned().collect::<Vec<_>>())
            .context("building relay route for group write")?
            .publish(event)
            .context("publishing group write")?;
        Ok((event_id, write))
    }

    /// Publish into one NIP-29 group and wait for relay settlement.
    pub(crate) async fn publish_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        builder: EventBuilder,
        keys: &Keys,
    ) -> Result<EventId> {
        let (event_id, write) =
            self.publish_groups_with_handle(std::iter::once(group.to_string()), builder, keys)?;
        write.settled(fava::all_terminal()).await.context("waiting for group write settlement")?;
        Ok(event_id)
    }

    /// Publish exact signed bytes to an exact relay set (plain NIP-01).
    pub(crate) fn publish_signed_to(
        &self,
        relays: Vec<RelayUrl>,
        event: &Event,
    ) -> Result<fava::Write> {
        if relays.is_empty() {
            anyhow::bail!("cannot publish {} without a configured relay", event.id);
        }
        self.fava
            .to(relays)
            .context("building relay route for signed publish")?
            .publish(event.clone())
            .context("publishing signed event")
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
    pub(crate) fn group_hosts(&self) -> Vec<RelayUrl> {
        self.relays.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests;
