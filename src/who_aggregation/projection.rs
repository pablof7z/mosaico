use super::WhoAggregation;
use crate::identity::SessionIdentity;
use crate::state::{Channel, Profile};

impl WhoAggregation {
    pub(crate) fn profile(&self, pubkey: &str) -> Option<&Profile> {
        self.profiles.get(pubkey)
    }

    pub(crate) fn session_identity(&self, pubkey: &str) -> Option<&SessionIdentity> {
        self.identities.get(pubkey)
    }

    pub(crate) fn workspace_path(&self, channel_h: &str) -> Option<&str> {
        self.workspace_paths.get(channel_h).map(String::as_str)
    }

    pub(crate) fn is_archived(&self, channel_h: &str) -> bool {
        self.channel(channel_h).is_some_and(Channel::is_archived)
    }

    pub(crate) fn is_root(&self, channel_h: &str) -> bool {
        self.channel(channel_h)
            .is_some_and(|channel| channel.parent.is_empty())
    }

    pub(crate) fn root_for_channel(&self, channel_h: &str) -> anyhow::Result<String> {
        let mut current = channel_h;
        for _ in 0..32 {
            let Some(channel) = self.channel(current) else {
                if current == channel_h && self.workspace_paths.contains_key(channel_h) {
                    return Ok(channel_h.to_string());
                }
                anyhow::bail!("workspace resolver: incomplete ancestry for channel {channel_h:?}");
            };
            if channel.parent.is_empty() {
                return Ok(channel.channel_h.clone());
            }
            current = &channel.parent;
        }
        anyhow::bail!("workspace resolver: cyclic ancestry for channel {channel_h:?}")
    }

    pub(crate) fn scope_contains(&self, current: &str, channel_h: &str) -> anyhow::Result<bool> {
        Ok(!self.is_archived(current)
            && !self.is_archived(channel_h)
            && (current == channel_h
                || (self.is_root(current) && self.root_for_channel(channel_h)? == current)))
    }

    pub(crate) fn display_slug(&self, pubkey: &str) -> Option<String> {
        self.session_identity(pubkey)
            .map(SessionIdentity::display_slug)
            .or_else(|| {
                self.profile(pubkey).map(|profile| {
                    crate::idref::session_handle_from_profile_name(
                        &profile.slug,
                        &profile.agent_slug,
                    )
                })
            })
            .filter(|slug| !slug.is_empty())
    }
}
