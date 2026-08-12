use super::Nip29Provider;
use crate::domain::{DomainEvent, Reaction};
use crate::fabric::NostrEventCodec;
use anyhow::Result;
use nostr::Keys;

impl Nip29Provider {
    /// Publish a NIP-25 kind:7 reaction through NMP's group door, gating the
    /// channel exactly like chat.
    ///
    /// Nothing is seeded locally. NMP injects an accepted row into the matching
    /// group observation, so local and remote reactions share one observed-row
    /// path.
    ///
    /// This path deliberately never enqueues inbox or rings a doorbell: a reaction
    /// is passive awareness surfaced only at the target's next turn-start hook.
    pub(crate) async fn publish_reaction_checked(
        &self,
        reaction: &Reaction,
        keys: &Keys,
    ) -> Result<String> {
        let builder = self.wire.encode(&DomainEvent::Reaction(reaction.clone()))?;
        let channel = reaction.channel.as_str();
        if channel.is_empty() {
            anyhow::bail!("a reaction must name the group it is published into");
        }
        self.verify_publish_scope(channel, &keys.public_key().to_hex(), true)
            .await?;
        Ok(self.nmp.publish_group(channel, builder, keys)?.to_hex())
    }
}
