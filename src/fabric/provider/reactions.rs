use super::Nip29Provider;
use crate::domain::{DomainEvent, Reaction};
use crate::fabric::nip29::readiness::{ChannelCtx, ChannelGate};
use crate::fabric::{NostrEventCodec, RawEnvelope};
use anyhow::Result;
use nostr_sdk::prelude::Keys;

impl Nip29Provider {
    /// Sign and publish a NIP-25 kind:7 reaction, gating the channel exactly like
    /// chat, then feed the signed event back through the materializer for local
    /// visibility. The relay does not echo to the publishing connection (same
    /// rationale as chat seeding), so without this the reacting daemon's own
    /// sessions would never see the reaction. Routing through `materialize()` keeps
    /// the single writer of `relay_reactions` and stays idempotent (PK = event id).
    ///
    /// This path deliberately never enqueues inbox or rings a doorbell: a reaction
    /// is passive awareness surfaced only at the target's next turn-start hook.
    pub(crate) async fn publish_reaction_checked(
        &self,
        reaction: &Reaction,
        keys: &Keys,
    ) -> Result<String> {
        let builder = self.wire.encode(&DomainEvent::Reaction(reaction.clone()))?;
        let signed = self.nmp.sign_event(builder, keys).await?;

        let channel = reaction.channel.as_str();
        if !channel.is_empty() {
            let reactor_pubkey = signed.pubkey.to_hex();
            let parent = super::readiness::stored_parent_hint(self, channel)?;
            let ctx = ChannelCtx {
                channel,
                expect_member: &reactor_pubkey,
                parent_hint: parent.as_deref(),
                name: None,
                repair_whitelisted_admins: true,
            };
            if matches!(self.ensure_channel_ready(ctx).await, ChannelGate::Degraded) {
                anyhow::bail!(
                    "publish_reaction_checked: channel {channel} is not verified (ChannelGate::Degraded) — refusing to publish"
                );
            }
        }

        let event_id = self.nmp.enqueue_group_event(&signed)?;
        // Seed locally through the single materializer writer.
        self.with_store(|store| {
            crate::fabric::materialize(&RawEnvelope::Nostr(signed.clone()), store);
        });
        Ok(event_id.to_hex())
    }
}
