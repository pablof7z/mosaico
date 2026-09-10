//! NIP-29 admin/member reads through the daemon's live Nostr runtime group view.
//!
//! `role='admin'` is the only management authority over a channel. No roster is
//! persisted or optimistically changed in Mosaico's SQLite store.

use super::*;

impl Store {
    /// Can this pubkey manage the channel? (role='admin')
    pub fn is_channel_admin(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self.group_projection().is_channel_admin(channel_h, pubkey))
    }

    /// Is this pubkey a member of the channel? (admin OR member)
    pub fn is_channel_member(&self, channel_h: &str, pubkey: &str) -> Result<bool> {
        Ok(self.group_projection().is_channel_member(channel_h, pubkey))
    }

    /// Has Nostr runtime supplied usable aggregate group state for this channel?
    pub fn group_state_available(&self, channel_h: &str) -> Result<bool> {
        Ok(self.group_projection().group_state_available(channel_h))
    }

    /// All members (admins and members) of a channel.
    pub fn list_channel_members(&self, channel_h: &str) -> Result<Vec<ChannelMember>> {
        Ok(self.group_projection().list_channel_members(channel_h))
    }

    /// Channels this pubkey belongs to in ANY role (admin or member). Used by the
    /// subscription planner to cover every channel a local/ordinal pubkey is in.
    pub fn list_channels_where_member(&self, pubkey: &str) -> Result<Vec<String>> {
        Ok(self.group_projection().list_channels_where_member(pubkey))
    }

    /// Channels this pubkey can manage (every channel where it is an admin).
    pub fn list_channels_where_admin(&self, pubkey: &str) -> Result<Vec<String>> {
        Ok(self.group_projection().list_channels_where_admin(pubkey))
    }

    /// Number of members (admins + members) in a channel.
    pub fn count_channel_members(&self, channel_h: &str) -> Result<u64> {
        Ok(self.group_projection().count_channel_members(channel_h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TestGroup, TestGroupDelivery};

    fn two_channel_store() -> Store {
        let store = Store::open_memory().unwrap();
        store.install_test_group_delivery(TestGroupDelivery::new([
            TestGroup::new("root")
                .metadata("general", "", "", 1)
                .admins(["A".to_string()])
                .members(["A".to_string(), "B".to_string()]),
            TestGroup::new("root/task")
                .metadata("task", "", "root", 2)
                .admins(["B".to_string()])
                .members(["B".to_string(), "C".to_string()]),
        ]));
        store
    }

    #[test]
    fn admin_and_member_predicates_route_through_group_projection() {
        let store = two_channel_store();
        assert!(store.is_channel_admin("root", "A").unwrap());
        assert!(
            !store.is_channel_admin("root", "B").unwrap(),
            "B is a member, not an admin"
        );
        assert!(store.is_channel_member("root", "B").unwrap());
        assert!(
            store.is_channel_member("root/task", "B").unwrap(),
            "an admin is a member"
        );
        assert!(!store.is_channel_member("root", "Z").unwrap());
        assert!(!store.is_channel_member("missing", "A").unwrap());
    }

    #[test]
    fn list_channel_members_emits_admins_then_plain_members() {
        let store = two_channel_store();
        let members = store.list_channel_members("root").unwrap();
        let roles: Vec<_> = members
            .iter()
            .map(|m| (m.pubkey.as_str(), m.role.as_str()))
            .collect();
        assert_eq!(roles, vec![("A", "admin"), ("B", "member")]);
        assert_eq!(store.count_channel_members("root").unwrap(), 2);
        assert!(store.list_channel_members("missing").unwrap().is_empty());
    }

    #[test]
    fn channel_membership_queries_cover_both_roles() {
        let store = two_channel_store();
        // B is an admin of root/task and a plain member of root.
        let mut where_member = store.list_channels_where_member("B").unwrap();
        where_member.sort();
        assert_eq!(where_member, vec!["root", "root/task"]);
        let mut where_admin = store.list_channels_where_admin("B").unwrap();
        where_admin.sort();
        assert_eq!(where_admin, vec!["root/task"]);
        let mut a_admin = store.list_channels_where_admin("A").unwrap();
        a_admin.sort();
        assert_eq!(a_admin, vec!["root"]);
        // A plain member with no admin role anywhere.
        assert!(store.list_channels_where_admin("C").unwrap().is_empty());
    }

    #[test]
    fn group_state_available_reflects_projection_presence() {
        let store = two_channel_store();
        assert!(store.group_state_available("root").unwrap());
        assert!(store.group_state_available("root/task").unwrap());
        assert!(!store.group_state_available("never-seen").unwrap());
    }
}
