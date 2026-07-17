use super::WhoAggregation;
use crate::identity::SessionIdentity;
use crate::state::{Channel, Profile, Session, Status};

impl WhoAggregation {
    pub(crate) fn profile(&self, pubkey: &str) -> Option<&Profile> {
        self.profiles.get(pubkey)
    }

    pub(crate) fn session_identity(&self, pubkey: &str) -> Option<&SessionIdentity> {
        self.identities.get(pubkey)
    }

    pub(crate) fn session(&self, pubkey: &str) -> Option<&Session> {
        self.sessions_by_pubkey.get(pubkey)
    }

    pub(crate) fn status_for(&self, pubkey: &str, channel_h: &str) -> Option<&Status> {
        self.statuses_for(channel_h)
            .iter()
            .find(|status| status.pubkey == pubkey)
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

    pub(crate) fn is_member(&self, channel_h: &str, pubkey: &str) -> bool {
        self.members_for(channel_h)
            .iter()
            .any(|member| member.pubkey == pubkey)
    }

    pub(crate) fn full_channel_ref(&self, channel_h: &str) -> anyhow::Result<String> {
        let mut parts = Vec::new();
        let mut current = channel_h;
        for _ in 0..32 {
            let Some(channel) = self.channel(current) else {
                anyhow::bail!("workspace resolver: incomplete ancestry for channel {channel_h:?}");
            };
            if channel.parent.is_empty() {
                let mut reference = vec![channel.channel_h.clone()];
                parts.reverse();
                reference.extend(parts);
                return Ok(reference.join("."));
            }
            parts.push(self.channel_name(current).to_string());
            current = &channel.parent;
        }
        anyhow::bail!("workspace resolver: cyclic ancestry for channel {channel_h:?}")
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

    pub(crate) fn pubkey_ref(&self, pubkey: &str, local_host: &str) -> String {
        let profile = self.profile(pubkey);
        let slug = profile
            .map(|profile| profile.slug.clone())
            .filter(|slug| !slug.is_empty())
            .unwrap_or_else(|| crate::util::pubkey_short(pubkey));
        let host = profile
            .map(|profile| profile.host.clone())
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| local_host.to_string());
        if profile.is_some_and(|profile| !profile.agent_slug.is_empty()) {
            slug
        } else {
            crate::idref::agent_ref_from(&slug, &host, local_host)
        }
    }
}
