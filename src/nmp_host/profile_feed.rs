//! Retained kind:0 profile feed (mosaico#837, slice 1).
//!
//! One retained NMP LiveQuery scoped to `kinds:[0], authors:[current member
//! set]`, pinned to the profile relays with `CacheMode::Agnostic` (kind:0 is
//! self-authenticating), drained into a small bounded per-pubkey profile map.
//! This replaces the per-boot `NmpViews` mirror for `Store::get_profile`: the
//! feed holds only the current member set's profiles, not a union of all
//! stored events, so a daemon restart no longer rebuilds a gigabyte mirror.
//!
//! NMP owns replaceable kind:0 resolution: a newer kind:0 for a pubkey arrives
//! as `Added(new)` plus `Removed(old_id)` in one frame. The feed upserts on
//! `Added` and removes on `Removed` only when the id matches the stored row,
//! so the map always reflects NMP's current latest per author -- mirroring
//! `NmpViews::observed_profile`, which reads NMP's current state directly.
//!
//! `set_members` is driven from `sync_subscriptions` (the daemon's single
//! coverage refresh point), which already computes the member set as the union
//! of the backend identity plus every channel's members and admins.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::JoinHandle;

use nmp::{ObservationCancel, RowDelta, Subscription};
use nostr::EventId;

use crate::nmp_views::observed_profile_from_row;
use crate::state::Profile;

/// A retained kind:0 profile feed scoped to the current member set.
pub(crate) struct ProfileFeed {
    host: Option<Arc<crate::nmp_host::NmpHost>>,
    members: RwLock<BTreeSet<String>>,
    profiles: Mutex<BTreeMap<String, Profile>>,
    pk_event: Mutex<BTreeMap<String, EventId>>,
    event_pk: Mutex<BTreeMap<EventId, String>>,
    active: Mutex<Option<ActiveFeed>>,
    #[cfg(test)]
    test_profiles: RwLock<Option<BTreeMap<String, Profile>>>,
}

struct ActiveFeed {
    cancel: ObservationCancel,
    drain: JoinHandle<()>,
}

impl Default for ProfileFeed {
    fn default() -> Self {
        Self {
            host: None,
            members: RwLock::new(BTreeSet::new()),
            profiles: Mutex::new(BTreeMap::new()),
            pk_event: Mutex::new(BTreeMap::new()),
            event_pk: Mutex::new(BTreeMap::new()),
            active: Mutex::new(None),
            #[cfg(test)]
            test_profiles: RwLock::new(None),
        }
    }
}

impl ProfileFeed {
    /// Construct a feed backed by a live NMP host. No observation is opened
    /// until [`Self::set_members`] supplies a member set.
    pub(crate) fn new(host: Arc<crate::nmp_host::NmpHost>) -> Self {
        Self {
            host: Some(host),
            members: RwLock::new(BTreeSet::new()),
            profiles: Mutex::new(BTreeMap::new()),
            pk_event: Mutex::new(BTreeMap::new()),
            event_pk: Mutex::new(BTreeMap::new()),
            active: Mutex::new(None),
            #[cfg(test)]
            test_profiles: RwLock::new(None),
        }
    }

    /// Synchronous read of the current profile for `pubkey`.
    pub(crate) fn profile(&self, pubkey: &str) -> Option<Profile> {
        #[cfg(test)]
        if let Some(test) = self
            .test_profiles
            .read()
            .expect("profile feed test slot poisoned")
            .as_ref()
        {
            return test.get(pubkey).cloned();
        }
        self.profiles
            .lock()
            .expect("profile feed map poisoned")
            .get(pubkey)
            .cloned()
    }

    /// Replace the member set and the underlying retained observation. A no-op
    /// when the set is unchanged, so coverage refreshes that recompute the same
    /// roster do not churn the relay. Profiles for pubkeys that left the set
    /// are dropped; the new observation re-delivers the new author set's
    /// current kind:0 rows. The previous drain is cancelled and detached.
    pub(crate) fn set_members(self: &Arc<Self>, members: BTreeSet<String>) {
        if *self.members.read().expect("profile feed members poisoned") == members {
            return;
        }
        let host = match self.host.as_ref() {
            Some(host) => host.clone(),
            None => {
                *self.members.write().expect("profile feed members poisoned") = members;
                return;
            }
        };
        self.drop_departed(&members);
        *self.members.write().expect("profile feed members poisoned") = members.clone();

        let query = match host.profile_feed_query(&members) {
            Ok(query) => query,
            Err(error) => {
                tracing::error!(?error, "profile feed query build failed");
                return;
            }
        };
        let subscription = match host.observe_query(query) {
            Ok(subscription) => subscription,
            Err(error) => {
                tracing::error!(?error, "profile feed observation open failed");
                return;
            }
        };
        let cancel = subscription.cancel_handle();
        let drain = spawn_drain(subscription, Arc::clone(self));
        let previous = self
            .active
            .lock()
            .expect("profile feed active slot poisoned")
            .replace(ActiveFeed { cancel, drain });
        if let Some(previous) = previous {
            // Cancel and detach: the old drain exits on recv error, releasing its
            // Arc clone. Joining here would block the async coverage refresh.
            previous.cancel.cancel();
        }
    }

    /// Install synthetic profiles for Store-logic tests that need `get_profile`
    /// data without driving a real NMP engine. Mirrors `NmpViews`' test relay
    /// delivery seam: a later install replaces the whole set atomically.
    #[cfg(test)]
    pub(crate) fn install_test_profiles(&self, profiles: Vec<Profile>) {
        let map = profiles
            .into_iter()
            .map(|profile| (profile.pubkey.clone(), profile))
            .collect();
        *self
            .test_profiles
            .write()
            .expect("profile feed test slot poisoned") = Some(map);
    }

    /// Apply one NMP row delta to the profile map. Extracted from the drain
    /// loop so the upsert/remove/latest-wins logic is testable in isolation.
    fn apply_delta(&self, delta: RowDelta) {
        match delta {
            RowDelta::Added(row) => {
                if row.event.kind.as_u16() != 0 {
                    return;
                }
                let Some(observed) = observed_profile_from_row(row) else {
                    return;
                };
                let pubkey = observed.profile.agent.pubkey.clone();
                let event_id = observed.row.event.id;
                let profile = observed.as_state_profile();
                // Track only the CURRENT event id per pubkey: a newer kind:0
                // supersedes the old, so the old id is retired from the reverse
                // index and a Removed for it becomes a no-op.
                let mut pk_event = self
                    .pk_event
                    .lock()
                    .expect("profile feed pk->event index poisoned");
                if let Some(old) = pk_event.insert(pubkey.clone(), event_id) {
                    self.event_pk
                        .lock()
                        .expect("profile feed event->pk index poisoned")
                        .remove(&old);
                }
                self.event_pk
                    .lock()
                    .expect("profile feed event->pk index poisoned")
                    .insert(event_id, pubkey.clone());
                self.profiles
                    .lock()
                    .expect("profile feed map poisoned")
                    .insert(pubkey, profile);
            }
            RowDelta::Removed(id) => {
                let pubkey = self
                    .event_pk
                    .lock()
                    .expect("profile feed event->pk index poisoned")
                    .remove(&id);
                if let Some(pubkey) = pubkey {
                    self.pk_event
                        .lock()
                        .expect("profile feed pk->event index poisoned")
                        .remove(&pubkey);
                    self.profiles
                        .lock()
                        .expect("profile feed map poisoned")
                        .remove(&pubkey);
                }
            }
            RowDelta::SourcesGrew { .. } => {}
        }
    }

    fn drop_departed(&self, members: &BTreeSet<String>) {
        self.profiles
            .lock()
            .expect("profile feed map poisoned")
            .retain(|pubkey, _| members.contains(pubkey));
        self.pk_event
            .lock()
            .expect("profile feed pk->event index poisoned")
            .retain(|pubkey, _| members.contains(pubkey));
        self.event_pk
            .lock()
            .expect("profile feed event->pk index poisoned")
            .retain(|_, pubkey| members.contains(pubkey));
    }
}

impl Drop for ProfileFeed {
    fn drop(&mut self) {
        if let Some(active) = self
            .active
            .lock()
            .expect("profile feed active slot poisoned")
            .take()
        {
            active.cancel.cancel();
            let _ = active.drain.join();
        }
    }
}

fn spawn_drain(subscription: Subscription, feed: Arc<ProfileFeed>) -> JoinHandle<()> {
    // The drain owns the subscription; on `recv()` error (engine shutdown or
    // cancel) it exits, which drops the subscription and withdraws the demand.
    std::thread::Builder::new()
        .name("nmp-profile-feed".to_string())
        .spawn(move || {
            while let Ok(frame) = subscription.recv() {
                for delta in frame.deltas {
                    feed.apply_delta(delta);
                }
            }
        })
        .expect("spawn profile feed drain")
}

#[cfg(test)]
mod tests;
