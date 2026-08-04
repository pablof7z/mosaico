use super::Nip29Provider;
use anyhow::Result;

impl Nip29Provider {
    /// Verify that an outbound event may target `channel` without changing
    /// channel state. Creation and membership repair belong exclusively to
    /// explicit create/join flows.
    ///
    /// Read entirely from the cache the retained group-records observation
    /// keeps current. The gate is on the publish hot path — once per status,
    /// chat message and reaction — and used to pay a bounded relay read there.
    ///
    /// "Not yet observed" is kept distinct from "observed and absent", because
    /// the two justify opposite answers and collapsing them is how a gate
    /// starts refusing a group that exists.
    pub(super) async fn verify_publish_scope(
        &self,
        channel: &str,
        signer: &str,
        require_member: bool,
    ) -> Result<()> {
        anyhow::ensure!(!channel.is_empty(), "publish: channel must not be empty");
        self.with_store(|store| {
            anyhow::ensure!(
                store.get_channel(channel)?.is_some(),
                "publish: channel {channel} does not exist; create it explicitly before publishing"
            );
            if require_member {
                anyhow::ensure!(
                    store.has_channel_membership_snapshot(channel)?,
                    "publish: no relay-signed roster observed for channel {channel} yet; \
                     membership cannot be verified"
                );
                anyhow::ensure!(
                    store.is_channel_member(channel, signer)?,
                    "publish: signer is not a member of channel {channel}; join it explicitly before publishing"
                );
            }
            Ok(())
        })
    }
}
