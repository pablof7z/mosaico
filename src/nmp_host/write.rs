//! NIP-29 writes and profile publication behind the Fava facade.
//!
//! Fava owns every signature, write receipt, and relay-delivery lifecycle.
//! Mosaico composes only unsigned event bodies and names their public author.

use std::collections::BTreeSet;

use anyhow::{Context, Result};
use nostr::event::{EventBuilder, EventId, UnsignedEvent};
use nostr::key::PublicKey;
use nostr::types::Timestamp;

use super::scrub::scrub_unsigned;
use super::NmpHost;

pub(crate) mod group_management;
pub(crate) use group_management::GroupUser;

impl NmpHost {
    /// Publish a draft into ONE NIP-29 group as the exact `author`.
    pub(crate) fn publish_group(
        &self,
        group: &str,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<EventId> {
        self.publish_groups(std::iter::once(group.to_string()), builder, author)
    }

    /// Publish a draft into EVERY named group at once as the exact `author`.
    pub(crate) fn publish_groups(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<EventId> {
        self.publish_groups_receipt(groups, builder, author)
    }

    pub(super) fn publish_groups_receipt(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<EventId> {
        let (event_id, _write) = self.publish_groups_with_handle(groups, builder, author)?;
        Ok(event_id)
    }

    fn publish_groups_with_handle(
        &self,
        groups: impl IntoIterator<Item = String>,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<(EventId, fava::Write)> {
        #[cfg(test)]
        if let Some(refusal) = self.test_io.take_write() {
            refusal?;
        }
        let groups: BTreeSet<String> = groups.into_iter().collect();
        let mut unsigned = build_unsigned(author, builder)?;
        // The group context row (`h` tag) belongs to the group mechanism, not
        // the caller. A draft that supplies its own `h` tag is refused rather
        // than silently overwritten.
        if unsigned
            .tags
            .iter()
            .any(|tag| tag.as_slice().first().map(String::as_str) == Some("h"))
        {
            anyhow::bail!("the `h` tag belongs to the group, not the caller");
        }
        for group in &groups {
            unsigned.tags.push(
                nostr::event::Tag::parse(["h", group])
                    .map_err(|error| anyhow::anyhow!("h tag: {error}"))?,
            );
        }
        unsigned.id = None;
        scrub_unsigned(&mut unsigned);
        unsigned.ensure_id();
        let event_id = unsigned.id.context("building group event id")?;
        let write = self
            .fava
            .to(self.relays.iter().cloned().collect::<Vec<_>>())
            .context("building relay route for group write")?
            .publish(unsigned)
            .context("publishing group write")?;
        Ok((event_id, write))
    }

    /// Publish into one NIP-29 group and wait for relay settlement.
    pub(crate) async fn publish_group_and_wait(
        self: &std::sync::Arc<Self>,
        group: &str,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<EventId> {
        let (event_id, write) =
            self.publish_groups_with_handle(std::iter::once(group.to_string()), builder, author)?;
        write
            .settled(fava::all_terminal())
            .await
            .context("waiting for group write settlement")?;
        Ok(event_id)
    }

    /// Publish a kind:0 copy to every configured app/indexer relay.
    pub(crate) fn enqueue_profile(
        &self,
        builder: EventBuilder,
        author: PublicKey,
    ) -> Result<EventId> {
        let unsigned = build_unsigned(author, builder)?;
        if unsigned.kind.as_u16() != 0 {
            anyhow::bail!(
                "profile enqueue requires kind:0, got {}",
                unsigned.kind.as_u16()
            );
        }
        let event_id = unsigned.id.context("building profile event id")?;
        self.fava
            .to(self.profile_relays.iter().cloned().collect::<Vec<_>>())
            .context("building relay route for profile write")?
            .publish(unsigned)
            .context("publishing profile write")?;
        Ok(event_id)
    }
}

/// Translate Mosaico's NIP-29 body builder into Fava's checked unsigned event
/// construction without involving a local signing key.
fn build_unsigned(author: PublicKey, builder: EventBuilder) -> Result<UnsignedEvent> {
    fava::AuthoredEventBuilder::from_parts(
        author,
        builder.kind,
        builder.created_at.unwrap_or_else(Timestamp::now),
        builder.tags.to_vec(),
        builder.content,
    )
    .build()
    .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests;
