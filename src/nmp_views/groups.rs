//! Stateless product projections over one caller-owned NMP group delivery.

use std::collections::BTreeSet;

use anyhow::Result;
use nmp::nip29::{GroupAvailability, GroupSnapshot};

use crate::state::{Channel, ChannelMember};

#[cfg(test)]
mod test_delivery;
#[cfg(test)]
pub(crate) use test_delivery::{TestGroup, TestGroupDelivery};

const MAX_CHANNEL_PARENT_DEPTH: usize = 16;

/// A transient, read-only projection of one complete NMP delivery.
///
/// This owns no durable state and applies no host-union or replacement rule.
/// NMP already made those decisions in `GroupSnapshot`; this type only converts
/// the delivered values into Mosaico's product vocabulary.
pub(crate) struct GroupProjection {
    groups: Vec<ProjectedGroup>,
}

#[derive(Clone)]
pub(super) struct ProjectedGroup {
    pub(super) id: String,
    pub(super) channel: Option<Channel>,
    pub(super) admins: BTreeSet<String>,
    pub(super) members: BTreeSet<String>,
    pub(super) availability: GroupAvailability,
}

impl GroupProjection {
    pub(crate) fn new(snapshots: &[GroupSnapshot]) -> Self {
        Self {
            groups: snapshots.iter().map(ProjectedGroup::from).collect(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_test_delivery(delivery: &TestGroupDelivery) -> Self {
        Self {
            groups: delivery.groups().to_vec(),
        }
    }

    #[cfg(test)]
    pub(crate) fn group_availability(&self, id: &str) -> Option<GroupAvailability> {
        self.group(id).map(|group| group.availability)
    }

    pub(crate) fn get_channel(&self, channel_h: &str) -> Option<Channel> {
        self.group(channel_h)
            .and_then(|group| group.channel.clone())
    }

    pub(crate) fn list_channels(&self) -> Vec<Channel> {
        self.groups
            .iter()
            .filter_map(|group| group.channel.clone())
            .collect()
    }

    pub(crate) fn channel_id_for_name(&self, parent: &str, name: &str) -> Option<String> {
        if parent.is_empty() || name.is_empty() {
            return None;
        }
        self.list_channels()
            .into_iter()
            .find(|channel| channel.parent == parent && channel.name == name)
            .map(|channel| channel.channel_h)
    }

    pub(crate) fn channel_parent(&self, channel_h: &str) -> Option<String> {
        self.get_channel(channel_h).map(|channel| channel.parent)
    }

    pub(crate) fn root_channel_of(&self, channel_h: &str) -> Result<Option<String>> {
        if self.get_channel(channel_h).is_none() {
            return Ok(None);
        }
        let mut current = channel_h.to_string();
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_CHANNEL_PARENT_DEPTH {
            if !seen.insert(current.clone()) {
                anyhow::bail!("channel parent cycle detected at {current}");
            }
            let Some(parent) = self.channel_parent(&current) else {
                return Ok(None);
            };
            if parent.is_empty() {
                return Ok(Some(current));
            }
            current = parent;
        }
        anyhow::bail!(
            "channel parent chain exceeds {MAX_CHANNEL_PARENT_DEPTH} links at {channel_h}"
        );
    }

    pub(crate) fn is_root_channel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .root_channel_of(channel_h)?
            .is_some_and(|root| root == channel_h))
    }

    pub(crate) fn is_subchannel(&self, channel_h: &str) -> Result<bool> {
        Ok(self
            .root_channel_of(channel_h)?
            .is_some_and(|root| root != channel_h))
    }

    pub(crate) fn list_root_channels(&self) -> Vec<Channel> {
        self.list_channels()
            .into_iter()
            .filter(|channel| channel.parent.is_empty())
            .collect()
    }

    pub(crate) fn list_child_channels(&self, parent: &str) -> Vec<Channel> {
        self.list_channels()
            .into_iter()
            .filter(|channel| channel.parent == parent)
            .collect()
    }

    pub(crate) fn list_channel_members(&self, channel_h: &str) -> Vec<ChannelMember> {
        let Some(group) = self.group(channel_h) else {
            return Vec::new();
        };
        group
            .admins
            .iter()
            .map(|pubkey| member(channel_h, pubkey, "admin"))
            .chain(
                group
                    .members
                    .difference(&group.admins)
                    .map(|pubkey| member(channel_h, pubkey, "member")),
            )
            .collect()
    }

    pub(crate) fn is_channel_admin(&self, channel_h: &str, pubkey: &str) -> bool {
        self.group(channel_h)
            .is_some_and(|group| group.admins.contains(pubkey))
    }

    pub(crate) fn is_channel_member(&self, channel_h: &str, pubkey: &str) -> bool {
        self.group(channel_h)
            .is_some_and(|group| group.admins.contains(pubkey) || group.members.contains(pubkey))
    }

    pub(crate) fn list_channels_where_admin(&self, pubkey: &str) -> Vec<String> {
        self.groups
            .iter()
            .filter(|group| group.admins.contains(pubkey))
            .map(|group| group.id.clone())
            .collect()
    }

    pub(crate) fn list_channels_where_member(&self, pubkey: &str) -> Vec<String> {
        self.groups
            .iter()
            .filter(|group| group.admins.contains(pubkey) || group.members.contains(pubkey))
            .map(|group| group.id.clone())
            .collect()
    }

    pub(crate) fn count_channel_members(&self, channel_h: &str) -> u64 {
        self.list_channel_members(channel_h).len() as u64
    }

    pub(crate) fn group_state_available(&self, channel_h: &str) -> bool {
        self.group(channel_h).is_some_and(|group| {
            matches!(
                group.availability,
                GroupAvailability::Ready | GroupAvailability::CachedOnly
            )
        })
    }

    fn group(&self, id: &str) -> Option<&ProjectedGroup> {
        self.groups.iter().find(|group| group.id == id)
    }
}

impl From<&GroupSnapshot> for ProjectedGroup {
    fn from(snapshot: &GroupSnapshot) -> Self {
        let channel = snapshot.metadata.as_ref().map(|metadata| {
            let parent = metadata
                .tags
                .iter()
                .find(|row| row.first().map(String::as_str) == Some("parent"))
                .and_then(|row| row.get(1))
                .cloned()
                .unwrap_or_default();
            let as_of = metadata.as_of.as_secs();
            Channel {
                channel_h: snapshot.id.clone(),
                name: metadata.name.clone().unwrap_or_default(),
                about: metadata.about.clone().unwrap_or_default(),
                parent,
                created_at: as_of,
                updated_at: as_of,
            }
        });
        Self {
            id: snapshot.id.clone(),
            channel,
            admins: snapshot
                .admins
                .iter()
                .map(|subject| subject.pubkey.to_hex())
                .collect(),
            members: snapshot
                .members
                .iter()
                .map(|subject| subject.pubkey.to_hex())
                .collect(),
            availability: snapshot.availability,
        }
    }
}

fn member(channel_h: &str, pubkey: &str, role: &str) -> ChannelMember {
    ChannelMember {
        channel_h: channel_h.to_string(),
        pubkey: pubkey.to_string(),
        role: role.to_string(),
    }
}
