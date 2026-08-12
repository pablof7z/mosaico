use super::Nip29Provider;
use anyhow::Result;

/// Exact, short-lived authority returned by one successful NMP group mutation.
///
/// This is not roster state and is never retained. It only bridges the command
/// that received NMP's terminal result to the immediately following write
/// while the daemon's retained observation catches up.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmedGroupScope {
    channel: String,
    signer: String,
    authority: ScopeAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeAuthority {
    GroupExists,
    Membership,
}

impl ConfirmedGroupScope {
    pub(crate) fn from_nmp_membership(channel: &str, signer: &str) -> Self {
        Self {
            channel: channel.to_string(),
            signer: signer.to_string(),
            authority: ScopeAuthority::Membership,
        }
    }

    pub(crate) fn from_nmp_removal(channel: &str, signer: &str) -> Self {
        Self {
            channel: channel.to_string(),
            signer: signer.to_string(),
            authority: ScopeAuthority::GroupExists,
        }
    }

    pub(super) fn require_membership(&self, channel: &str, signer: &str) -> Result<()> {
        anyhow::ensure!(
            self.permits(channel, signer, true),
            "confirmed membership for {}/{} does not authorize {}/{}",
            self.channel,
            crate::util::pubkey_short(&self.signer),
            channel,
            crate::util::pubkey_short(signer),
        );
        Ok(())
    }

    pub(super) fn permits(&self, channel: &str, signer: &str, require_member: bool) -> bool {
        self.channel == channel
            && self.signer == signer
            && (!require_member || self.authority == ScopeAuthority::Membership)
    }
}

impl Nip29Provider {
    /// Verify that an outbound event may target `channel` without changing
    /// channel state. Creation and membership repair belong exclusively to
    /// explicit create/join flows.
    ///
    /// Read entirely from the retained NMP group observation. The gate is on
    /// the publish hot path — once per status, chat message and reaction — and
    /// must not pay a second relay read there.
    ///
    /// Overall acquisition may still be incomplete while the current snapshot
    /// positively names this signer. Incomplete state cannot prove absence or
    /// a complete roster, but it does not invalidate a present member row.
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
                    store.is_channel_member(channel, signer)?,
                    "publish: signer is not a member of channel {channel}; join it explicitly before publishing"
                );
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ConfirmedGroupScope;

    #[test]
    fn a_removal_result_can_expire_status_but_never_authorize_live_publication() {
        let removed = ConfirmedGroupScope::from_nmp_removal("channel", "signer");

        assert!(removed.permits("channel", "signer", false));
        assert!(!removed.permits("channel", "signer", true));
        assert!(!removed.permits("other", "signer", false));
        assert!(!removed.permits("channel", "other", false));

        let admitted = ConfirmedGroupScope::from_nmp_membership("channel", "signer");
        assert!(admitted.permits("channel", "signer", false));
        assert!(admitted.permits("channel", "signer", true));
    }
}
