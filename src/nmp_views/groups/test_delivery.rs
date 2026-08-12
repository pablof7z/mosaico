//! Complete NMP deliveries for unit tests that do not run a relay.
//!
//! Building may be incremental, but installation is atomic and replaces the
//! whole delivery. This module contains no host-folding or persistence rule.

use std::collections::BTreeSet;

use nmp::nip29::GroupAvailability;

use super::ProjectedGroup;
use crate::state::Channel;

#[derive(Clone, Default)]
pub(crate) struct TestGroupDelivery {
    groups: Vec<ProjectedGroup>,
}

pub(crate) struct TestGroup {
    projected: ProjectedGroup,
}

impl TestGroupDelivery {
    pub(crate) fn new(groups: impl IntoIterator<Item = TestGroup>) -> Self {
        let mut groups = groups
            .into_iter()
            .map(|group| group.projected)
            .collect::<Vec<_>>();
        groups.sort_by(|left, right| left.id.cmp(&right.id));
        Self { groups }
    }

    pub(super) fn groups(&self) -> &[ProjectedGroup] {
        &self.groups
    }
}

impl TestGroup {
    pub(crate) fn new(id: &str) -> Self {
        Self {
            projected: ProjectedGroup {
                id: id.to_string(),
                channel: None,
                admins: BTreeSet::new(),
                members: BTreeSet::new(),
                availability: GroupAvailability::Ready,
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

    pub(crate) fn availability(mut self, availability: GroupAvailability) -> Self {
        self.projected.availability = availability;
        self
    }
}
