//! NIP-29 admin/member reads through the daemon's live NMP group view.
//!
//! `role='admin'` is the only management authority over a channel. No roster is
//! persisted or optimistically changed in Mosaico's SQLite store.

use super::*;

impl Store {
    /// Can this pubkey manage the channel? (role='admin')
    pub fn is_channel_admin(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.is_channel_admin(channel_h, pubkey)))
    }

    /// Is this pubkey a member of the channel? (admin OR member)
    pub fn is_channel_member(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.is_channel_member(channel_h, pubkey)))
    }

    /// Has NMP supplied usable aggregate group state for this channel?
    pub fn group_state_available(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.group_state_available(channel_h)))
    }

    /// All members (admins and members) of a channel.
    pub fn list_channel_members(&self, channel_h: &str) -> Result<Vec<ChannelMember>> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.list_channel_members(channel_h)))
    }

    /// Channels this pubkey belongs to in ANY role (admin or member). Used by the
    /// subscription planner to cover every channel a local/ordinal pubkey is in.
    pub fn list_channels_where_member(&self, pubkey: &str) -> Result<Vec<String>> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.list_channels_where_member(pubkey)))
    }

    /// Channels this pubkey can manage (every channel where it is an admin).
    pub fn list_channels_where_admin(&self, pubkey: &str) -> Result<Vec<String>> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.list_channels_where_admin(pubkey)))
    }

    /// Number of members (admins + members) in a channel.
    pub fn count_channel_members(&self, channel_h: &str) -> Result<u64> {
        Ok(self
            .nmp_views
            .with_groups(|groups| groups.count_channel_members(channel_h)))
    }
}
