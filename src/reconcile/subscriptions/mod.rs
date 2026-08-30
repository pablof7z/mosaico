//! The daemon's single live-query policy.
//!
//! The daemon supplies a complete coverage snapshot. This module computes the
//! desired broad `#h` observation and returns the required Open/Replace/Close
//! effects. Fava owns relay work behind the host seam.
//!
//! (mosaico#837) The N per-channel content observations collapsed into ONE broad
//! bounded `GroupContentsMulti` `#h` over all known channels. The global
//! kind:39000 group-metadata observation moved to the retained `group_records`
//! observation, and per-author kind:0 profiles moved to the retained
//! [`ProfileFeed`](crate::fava_host::ProfileFeed). What remains here is the broad
//! `#h` alone — every other live query is a retained observation owned elsewhere.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What an observation is asking about, named rather than spelled.
///
/// The distinction between the NIP-29 variants and the rest is not cosmetic.
/// A NIP-29 question is answered by a RELAY, not by the network: the `h` tag
/// is a label and two relays hosting the same group id are two independent
/// groups. Fava mints those reads itself
/// (`fava_nip29::Group::read` / `fava_nip29::groups_where_at`) precisely so
/// both host-scoping axes get stamped — `ReadRouting::Explicit` for which
/// relays are ASKED and `CacheMode::Strict` for which cached rows may ANSWER.
/// Naming the group here rather than carrying a raw `('h', value)` tag is
/// what lets `FavaHost` route each variant to the door that owns it; Fava's own
/// door REFUSES a caller-supplied `#h` constraint, so a raw tag could not be
/// handed to it at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubscriptionQuery {
    /// The contents of SEVERAL groups at once, scoped by a multi-value `#h`.
    /// The single broad observation that replaces N per-channel
    /// `GroupContents` (mosaico#837). Self-authenticating content
    /// (kinds 9/7/30315/9000) needs no per-host branching — one `#h`-bound
    /// demand over all group relays is equivalent to `Group::read`'s per-host
    /// branches for these kinds.
    GroupContentsMulti {
        groups: BTreeSet<String>,
        kinds: BTreeSet<u16>,
    },
    /// Events of these kinds referencing one event id in an `e` tag.
    References {
        event_id: String,
        kinds: BTreeSet<u16>,
    },
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
    /// when no local session has joined them. (Their kind:39000/39001/39002
    /// records ride the retained `group_records` observation, not this
    /// reconciler.)
    pub group_state_channels: BTreeSet<String>,
    pub addressed_pubkeys: BTreeSet<String>,
    pub profile_pubkeys: BTreeSet<String>,
    pub archived_channels: BTreeSet<String>,
    pub sessions: BTreeMap<String, BTreeSet<String>>,
}

/// The single broad `#h` observation id (mosaico#837). One bounded
/// `GroupContentsMulti` over all known channels replaces the former N
/// per-channel `ChannelH` observations plus the global kind:9000 observation
/// (kind 9000 carries `#h`, so it rides the same demand).
pub(crate) const BROAD_H_ID: &str = "mosaico-broad-h";

#[derive(Clone, Default)]
pub struct SubscriptionReconciler {
    /// The current broad `#h` observation's group set (`None` = not open).
    applied_h_groups: Option<BTreeSet<String>>,
    desired_h_groups: BTreeSet<String>,
}

impl SubscriptionReconciler {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn plan(&mut self, snapshot: &CoverageSnapshot) -> Vec<SubEffect> {
        let desired_h = desired_h_groups(snapshot);
        let mut effects = Vec::new();

        // The broad #h observation: open, replace (on set change), or close.
        match (&self.applied_h_groups, &desired_h) {
            (None, groups) if !groups.is_empty() => effects.push(SubEffect::Open {
                id: BROAD_H_ID.to_string(),
                query: broad_h_query(groups),
            }),
            (Some(applied), groups) if applied != groups && !groups.is_empty() => {
                effects.push(SubEffect::Replace {
                    id: BROAD_H_ID.to_string(),
                    query: broad_h_query(groups),
                })
            }
            (Some(_), groups) if groups.is_empty() => effects.push(SubEffect::Close {
                id: BROAD_H_ID.to_string(),
            }),
            _ => {}
        }

        self.desired_h_groups = desired_h;
        effects
    }

    pub fn confirm(&mut self, effect: &SubEffect) {
        match effect {
            SubEffect::Open { id, .. } | SubEffect::Replace { id, .. } if id == BROAD_H_ID => {
                self.applied_h_groups = Some(self.desired_h_groups.clone());
            }
            SubEffect::Close { id } if id == BROAD_H_ID => {
                self.applied_h_groups = None;
            }
            _ => {}
        }
    }

    /// A channel is covered when the broad `#h` observation's group set
    /// currently includes it.
    pub fn covers_channel(&self, channel: &str) -> bool {
        self.applied_h_groups
            .as_ref()
            .is_some_and(|groups| groups.contains(channel))
    }

    #[cfg(test)]
    fn applied_h_groups(&self) -> Option<&BTreeSet<String>> {
        self.applied_h_groups.as_ref()
    }
}

/// The set of channels the broad `#h` observation must cover: every daemon and
/// session channel, minus archived. Sessions no longer refcount per-channel —
/// the union feeds one observation, and joining/leaving a channel replaces it
/// with the new set.
fn desired_h_groups(snapshot: &CoverageSnapshot) -> BTreeSet<String> {
    let mut groups = BTreeSet::new();
    for channel in snapshot
        .daemon_channels
        .difference(&snapshot.archived_channels)
    {
        groups.insert(channel.clone());
    }
    for channels in snapshot.sessions.values() {
        for channel in channels.difference(&snapshot.archived_channels) {
            groups.insert(channel.clone());
        }
    }
    groups
}

fn broad_h_query(groups: &BTreeSet<String>) -> SubscriptionQuery {
    use crate::fabric::nip29::wire::{KIND_CHAT, KIND_GROUP_PUT_USER, KIND_REACTION, KIND_STATUS};
    SubscriptionQuery::GroupContentsMulti {
        groups: groups.clone(),
        kinds: BTreeSet::from([KIND_CHAT, KIND_REACTION, KIND_STATUS, KIND_GROUP_PUT_USER]),
    }
}
