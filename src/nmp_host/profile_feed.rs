//! Retained kind:0 profile feed (mosaico#837, slice 1).
//!
//! One retained fava `Observation` scoped to `kinds:[0], authors:[current member
//! set]`, draining into a small bounded per-pubkey profile map. Each snapshot
//! delivery resets the map from the complete current event set.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use fava::{EventValue, Observation, ObservationClosed};
use nostr::event::EventId;

use crate::nmp_views::observed_profile_from_row;
use crate::row::Row;
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
    // The drain task owns the Observation; aborting the task drops it,
    // which calls Observation::Drop → close().
    _task: tokio::task::JoinHandle<()>,
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
    /// Construct a feed backed by a live fava host.
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

    /// Every profile in the current member set.
    pub(crate) fn all_profiles(&self) -> Vec<Profile> {
        #[cfg(test)]
        if let Some(test) = self
            .test_profiles
            .read()
            .expect("profile feed test slot poisoned")
            .as_ref()
        {
            return test.values().cloned().collect();
        }
        self.profiles
            .lock()
            .expect("profile feed map poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Replace the member set and the underlying retained observation.
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

        // Open observation on the tokio runtime via a new OS thread so that
        // Handle::block_on is not called from within an async task.
        let observation = match super::observe_sync(&host.fava, query) {
            Ok(obs) => obs,
            Err(error) => {
                tracing::error!(?error, "profile feed observation open failed");
                return;
            }
        };

        // Dropping the previous ActiveFeed aborts its task, which drops
        // the Observation, which calls Observation::Drop → close().
        let task = spawn_drain(observation, Arc::clone(self));
        self.active
            .lock()
            .expect("profile feed active slot poisoned")
            .replace(ActiveFeed { _task: task });
    }

    /// Install synthetic profiles for Store-logic tests.
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

    /// Apply one delta (Added or Removed) to the profile map.
    fn apply_delta(&self, row: Option<Row>, removed: Option<EventId>) {
        if let Some(id) = removed {
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
            return;
        }
        // Added
        let Some(row) = row else { return };
        if row.event.kind.as_u16() != 0 {
            return;
        }
        let Some(observed) = observed_profile_from_row(row) else {
            return;
        };
        let pubkey = observed.profile.agent.pubkey.clone();
        let event_id = observed.row.event.id;
        let profile = observed.as_state_profile();
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
        // Dropping the ActiveFeed drops the JoinHandle, which aborts the task.
        // The task owns the Observation; aborting it triggers Observation::drop → close().
        self.active
            .lock()
            .expect("profile feed active slot poisoned")
            .take();
    }
}

fn spawn_drain(
    mut observation: Observation,
    feed: Arc<ProfileFeed>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Track the previous event set to compute deltas.
        let mut prev_ids: BTreeSet<EventId> = BTreeSet::new();
        loop {
            match observation.changed().await {
                Ok(snapshot) => {
                    let curr_events: BTreeMap<EventId, Row> = snapshot
                        .events
                        .iter()
                        .filter_map(|record| match record.event() {
                            EventValue::Signed(e) if e.kind.as_u16() == 0 => {
                                Some((e.id, Row { event: e.clone() }))
                            }
                            _ => None,
                        })
                        .collect();
                    let curr_ids: BTreeSet<EventId> = curr_events.keys().copied().collect();
                    // Removed = in prev but not in curr
                    for removed_id in prev_ids.difference(&curr_ids) {
                        feed.apply_delta(None, Some(*removed_id));
                    }
                    // Added = in curr but not in prev
                    for added_id in curr_ids.difference(&prev_ids) {
                        if let Some(row) = curr_events.get(added_id).cloned() {
                            feed.apply_delta(Some(row), None);
                        }
                    }
                    prev_ids = curr_ids;
                }
                Err(ObservationClosed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests;
