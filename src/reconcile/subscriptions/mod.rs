//! Refcounted, per-entity live-query policy.
//!
//! The daemon supplies a complete coverage snapshot. This module computes the
//! desired narrow observations and returns only the required open/close effects.
//! Ownership counts stay explicit and local; NMP owns relay work behind the host
//! seam.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubscriptionQuery {
    pub kinds: BTreeSet<u16>,
    pub authors: BTreeSet<String>,
    pub tag: Option<(char, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SubEffect {
    Open {
        id: String,
        query: SubscriptionQuery,
    },
    Close {
        id: String,
    },
    Replace {
        id: String,
        query: SubscriptionQuery,
    },
}

#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageSnapshot {
    pub daemon_channels: BTreeSet<String>,
    /// Known groups whose relay-authored metadata and roster stay hydrated even
    /// when no local session has joined them.
    pub group_state_channels: BTreeSet<String>,
    pub addressed_pubkeys: BTreeSet<String>,
    pub profile_pubkeys: BTreeSet<String>,
    pub archived_channels: BTreeSet<String>,
    pub sessions: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Space {
    GlobalKind,
    ChannelH,
    GroupStateD,
    PubkeyP,
    ProfileAuthor,
}

type SubKey = (Space, String);

#[derive(Clone, Default)]
pub struct SubscriptionReconciler {
    applied: BTreeSet<SubKey>,
    desired_owners: BTreeMap<SubKey, usize>,
}

impl SubscriptionReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plan(&mut self, snapshot: &CoverageSnapshot) -> Vec<SubEffect> {
        let desired = desired_owners(snapshot);
        let mut effects = Vec::new();

        for key in desired.keys() {
            if !self.applied.contains(key) {
                effects.push(open_effect(key));
            }
        }
        for key in &self.applied {
            if !desired.contains_key(key) {
                effects.push(SubEffect::Close { id: sub_id(key) });
            }
        }

        self.desired_owners = desired;
        effects
    }

    pub fn confirm(&mut self, effect: &SubEffect) {
        match effect {
            SubEffect::Open { id, .. } | SubEffect::Replace { id, .. } => {
                if let Some(key) = self
                    .desired_owners
                    .keys()
                    .find(|key| sub_id(key) == *id)
                    .cloned()
                {
                    self.applied.insert(key);
                }
            }
            SubEffect::Close { id } => {
                if let Some(key) = self.applied.iter().find(|key| sub_id(key) == *id).cloned() {
                    self.applied.remove(&key);
                }
            }
        }
    }

    pub fn covers_channel(&self, channel: &str) -> bool {
        self.applied
            .contains(&(Space::ChannelH, channel.to_string()))
    }

    #[cfg(test)]
    fn owner_count(&self, space: Space, entity: &str) -> usize {
        self.desired_owners
            .get(&(space, entity.to_string()))
            .copied()
            .unwrap_or(0)
    }
}

fn desired_owners(snapshot: &CoverageSnapshot) -> BTreeMap<SubKey, usize> {
    let mut desired = BTreeMap::new();
    add_owner(
        &mut desired,
        (
            Space::GlobalKind,
            crate::fabric::nip29::wire::KIND_GROUP_PUT_USER.to_string(),
        ),
    );
    add_owner(
        &mut desired,
        (
            Space::GlobalKind,
            crate::fabric::nip29::wire::KIND_GROUP_METADATA.to_string(),
        ),
    );

    for channel in snapshot
        .daemon_channels
        .difference(&snapshot.archived_channels)
    {
        add_channel_owner(&mut desired, channel);
    }
    for channel in snapshot
        .group_state_channels
        .difference(&snapshot.archived_channels)
    {
        add_owner(&mut desired, (Space::GroupStateD, channel.clone()));
    }
    for channels in snapshot.sessions.values() {
        for channel in channels.difference(&snapshot.archived_channels) {
            add_channel_owner(&mut desired, channel);
        }
    }
    for pubkey in &snapshot.addressed_pubkeys {
        add_owner(&mut desired, (Space::PubkeyP, pubkey.clone()));
    }
    for pubkey in &snapshot.profile_pubkeys {
        add_owner(&mut desired, (Space::ProfileAuthor, pubkey.clone()));
    }
    desired
}

fn add_channel_owner(owners: &mut BTreeMap<SubKey, usize>, channel: &str) {
    add_owner(owners, (Space::ChannelH, channel.to_string()));
    add_owner(owners, (Space::GroupStateD, channel.to_string()));
}

fn add_owner(owners: &mut BTreeMap<SubKey, usize>, key: SubKey) {
    *owners.entry(key).or_default() += 1;
}

fn open_effect(key: &SubKey) -> SubEffect {
    SubEffect::Open {
        id: sub_id(key),
        query: sub_query(key),
    }
}

fn sub_id((space, entity): &SubKey) -> String {
    match space {
        Space::GlobalKind => format!("mosaico-global-kind-{entity}"),
        Space::ChannelH => format!("mosaico-h-{entity}"),
        Space::GroupStateD => format!("mosaico-gstate-{entity}"),
        Space::PubkeyP => format!("mosaico-p-{entity}"),
        Space::ProfileAuthor => format!("mosaico-profile-{entity}"),
    }
}

fn sub_query((space, entity): &SubKey) -> SubscriptionQuery {
    use crate::fabric::nip29::wire::{
        KIND_CHAT, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS, KIND_GROUP_METADATA, KIND_STATUS,
    };
    match space {
        Space::GlobalKind => SubscriptionQuery {
            kinds: BTreeSet::from([entity.parse().expect("global kind is numeric")]),
            authors: BTreeSet::new(),
            tag: None,
        },
        Space::ChannelH => SubscriptionQuery {
            kinds: BTreeSet::from([KIND_CHAT, KIND_STATUS]),
            authors: BTreeSet::new(),
            tag: Some(('h', entity.clone())),
        },
        Space::GroupStateD => SubscriptionQuery {
            kinds: BTreeSet::from([KIND_GROUP_METADATA, KIND_GROUP_ADMINS, KIND_GROUP_MEMBERS]),
            authors: BTreeSet::new(),
            tag: Some(('d', entity.clone())),
        },
        Space::PubkeyP => SubscriptionQuery {
            kinds: BTreeSet::from([KIND_CHAT]),
            authors: BTreeSet::new(),
            tag: Some(('p', entity.clone())),
        },
        Space::ProfileAuthor => SubscriptionQuery {
            kinds: BTreeSet::from([crate::fabric::nip29::wire::KIND_PROFILE]),
            authors: BTreeSet::from([entity.clone()]),
            tag: None,
        },
    }
}
