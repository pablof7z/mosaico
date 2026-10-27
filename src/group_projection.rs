//! Stateless product projection of one complete Nostr runtime group subscription level.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;

use crate::group_snapshot::GroupSnapshot;
use crate::state::{Channel, ChannelMember};

const MAX_CHANNEL_PARENT_DEPTH: usize = 16;

/// Groups keyed by id, so per-channel reads are lookups.
#[derive(Default)]
pub(crate) struct GroupProjection {
    groups: BTreeMap<String, ProjectedGroup>,
}

#[derive(Clone)]
struct ProjectedGroup {
    id: String,
    channel: Option<Channel>,
    admins: BTreeSet<String>,
    members: BTreeSet<String>,
}

impl GroupProjection {
    pub(crate) fn new(snapshots: &[GroupSnapshot]) -> Self {
        Self {
            groups: snapshots
                .iter()
                .map(|snapshot| (snapshot.id.clone(), ProjectedGroup::from(snapshot)))
                .collect(),
        }
    }

    pub(crate) fn get_channel(&self, channel_h: &str) -> Option<Channel> {
        self.channel(channel_h).cloned()
    }

    pub(crate) fn list_channels(&self) -> Vec<Channel> {
        self.channels().cloned().collect()
    }

    pub(crate) fn channel_id_for_name(&self, parent: &str, name: &str) -> Option<String> {
        if parent.is_empty() || name.is_empty() {
            return None;
        }
        self.channels()
            .find(|channel| channel.parent == parent && channel.name == name)
            .map(|channel| channel.channel_h.clone())
    }

    pub(crate) fn channel_parent(&self, channel_h: &str) -> Option<String> {
        self.channel(channel_h).map(|channel| channel.parent.clone())
    }

    pub(crate) fn root_channel_of(&self, channel_h: &str) -> Result<Option<String>> {
        if self.channel(channel_h).is_none() {
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
        )
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
        self.channels()
            .filter(|channel| channel.parent.is_empty())
            .cloned()
            .collect()
    }

    pub(crate) fn list_child_channels(&self, parent: &str) -> Vec<Channel> {
        self.channels()
            .filter(|channel| channel.parent == parent)
            .cloned()
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
            .values()
            .filter(|group| group.admins.contains(pubkey))
            .map(|group| group.id.clone())
            .collect()
    }

    /// Every group any of `pubkeys` is an admin or member of, in one scan.
    pub(crate) fn channels_involving(&self, pubkeys: &BTreeSet<String>) -> BTreeSet<String> {
        self.groups
            .values()
            .filter(|group| {
                !group.admins.is_disjoint(pubkeys) || !group.members.is_disjoint(pubkeys)
            })
            .map(|group| group.id.clone())
            .collect()
    }

    pub(crate) fn count_channel_members(&self, channel_h: &str) -> u64 {
        self.list_channel_members(channel_h).len() as u64
    }

    pub(crate) fn group_state_available(&self, channel_h: &str) -> bool {
        self.group(channel_h).is_some()
    }

    fn group(&self, id: &str) -> Option<&ProjectedGroup> {
        self.groups.get(id)
    }

    fn channel(&self, id: &str) -> Option<&Channel> {
        self.group(id).and_then(|group| group.channel.as_ref())
    }

    fn channels(&self) -> impl Iterator<Item = &Channel> {
        self.groups.values().filter_map(|group| group.channel.as_ref())
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

#[cfg(test)]
#[derive(Clone, Default)]
pub(crate) struct TestGroupDelivery {
    groups: BTreeMap<String, ProjectedGroup>,
}

#[cfg(test)]
pub(crate) struct TestGroup {
    projected: ProjectedGroup,
}

#[cfg(test)]
impl TestGroupDelivery {
    pub(crate) fn new(groups: impl IntoIterator<Item = TestGroup>) -> Self {
        Self {
            groups: groups
                .into_iter()
                .map(|group| (group.projected.id.clone(), group.projected))
                .collect(),
        }
    }

    pub(crate) fn projection(&self) -> GroupProjection {
        GroupProjection {
            groups: self.groups.clone(),
        }
    }
}

#[cfg(test)]
impl TestGroup {
    pub(crate) fn new(id: &str) -> Self {
        Self {
            projected: ProjectedGroup {
                id: id.to_string(),
                channel: None,
                admins: BTreeSet::new(),
                members: BTreeSet::new(),
            },
        }
    }

    pub(crate) fn metadata(mut self, name: &str, about: &str, parent: &str, as_of: u64) -> Self {
        self.projected.channel = Some(Channel {
            channel_h: self.projected.id.clone(),
            name: name.to_string(),
            about: about.to_string(),
            parent: parent.to_string(),
            created_at: as_of,
            updated_at: as_of,
        });
        self
    }

    pub(crate) fn admins(mut self, admins: impl IntoIterator<Item = String>) -> Self {
        self.projected.admins = admins.into_iter().collect();
        self
    }

    pub(crate) fn members(mut self, members: impl IntoIterator<Item = String>) -> Self {
        self.projected.members = members.into_iter().collect();
        self
    }
}
