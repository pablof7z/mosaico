//! The member roster union: who is on each channel, how they are named, and
//! what we know about them beyond the roster row itself.
//!
//! Separated from the rest of the capture because member identity carries its
//! own resolution problem: a roster entry is only a pubkey, and turning that
//! into something an agent can address depends on a `kind:0` we may not have.
//! The `has_handle` set records which of those resolutions actually landed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// The member roster union source: per-channel roster pubkeys, the resolved
/// display ref for every pubkey that can appear, and the backend-pubkey set.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MembersInput {
    /// Per-channel roster as `pubkey -> role` (`admin`/`member`). The role is
    /// retained as relay state; rendered awareness exposes the member identity,
    /// status, and liveness only.
    pub(in crate::fabric_context) roster: BTreeMap<String, BTreeMap<String, String>>,
    pub(in crate::fabric_context) refs: BTreeMap<String, String>,
    #[serde(default)]
    pub(in crate::fabric_context) agent_slugs: BTreeMap<String, String>,
    pub(in crate::fabric_context) backend: BTreeSet<String>,
    /// channel_h -> (pubkey -> latest kind:9 message time). Activity-derived
    /// presence folded in alongside kind:30315 heartbeat statuses.
    #[serde(default)]
    pub(in crate::fabric_context) activity: BTreeMap<String, BTreeMap<String, u64>>,
    /// Pubkeys with a resolvable kind:0 handle (profile slug non-empty). A roster
    /// member outside this set can only render as a raw npub fallback, so it is
    /// omitted from member rows and queued for a profile refetch instead.
    #[serde(default)]
    pub(in crate::fabric_context) has_handle: BTreeSet<String>,
}

impl MembersInput {
    /// Whether `pubkey` resolved to a real kind:0 handle at capture time. A
    /// member without one has no name an agent could address, so awareness drops
    /// it rather than printing an npub nobody can act on.
    pub(in crate::fabric_context) fn has_handle(&self, pubkey: &str) -> bool {
        self.has_handle.contains(pubkey)
    }

    /// Latest observed kind:9 message time for `pubkey` in `channel`, if any.
    pub(in crate::fabric_context) fn activity_at(
        &self,
        channel: &str,
        pubkey: &str,
    ) -> Option<u64> {
        self.activity.get(channel)?.get(pubkey).copied()
    }
}
