//! Lifecycle-owned presentation state accumulated from fava deliveries.
//!
//! This is deliberately process-local. It owns no replacement, relay-union,
//! freshness, or persistence rule: fava supplies those answers. Mosaico only
//! retains the latest delivered values while their observations are alive.

use std::sync::{Arc, Mutex, RwLock};

#[cfg(test)]
use nostr::event::EventId;

use crate::group_snapshot::GroupSnapshot;
use crate::recent_rows::RecentRows;
use crate::row::Row;
use crate::row::RowDelta;

mod groups;
pub(crate) use groups::GroupProjection;
#[cfg(test)]
pub(crate) use groups::{TestGroup, TestGroupDelivery};
mod rows;
use rows::RowViews;
mod profile_status;
mod relay;
pub(crate) use relay::observed_profile_from_row;
#[cfg(test)]
mod test_delivery;
#[cfg(test)]
pub(crate) use test_delivery::TestRelayDelivery;

#[derive(Default)]
pub(crate) struct ObservedViews {
    group_snapshots: RwLock<Vec<GroupSnapshot>>,
    #[cfg(test)]
    test_group_delivery: RwLock<Option<TestGroupDelivery>>,
    #[cfg(test)]
    test_relay_delivery: RwLock<Option<TestRelayDelivery>>,
    rows: Mutex<RowViews>,
    recent_rows: Arc<RecentRows>,
}

#[derive(Default)]
pub(crate) struct RowTransition {
    pub(crate) removed: Vec<DepartedRow>,
    pub(crate) entered: Vec<EnteredRow>,
    pub(crate) added: Vec<Row>,
}

/// One exact Row departing one retained observation.
///
/// A Row can remain in the canonical union through another observation, so a
/// bare event id cannot express the channel scope whose current view changed.
pub(crate) struct DepartedRow {
    pub(crate) observation_id: String,
    pub(crate) row: Row,
}

/// One exact Row entering one retained observation.
///
/// This is distinct from [`RowTransition::added`], which reports only the first
/// addition to the canonical row union. Observation-scoped products such as
/// status must react to every channel edge, including a Row already owned by
/// another observation.
pub(crate) struct EnteredRow {
    pub(crate) observation_id: String,
    pub(crate) row: Row,
}

impl RowTransition {
    pub(crate) fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.entered.is_empty() && self.added.is_empty()
    }
}

impl ObservedViews {
    /// Replace the retained group snapshot set from the group-records drain.
    pub(crate) fn set_group_snapshots(&self, snapshots: Vec<GroupSnapshot>) {
        *self
            .group_snapshots
            .write()
            .expect("observed group snapshots poisoned") = snapshots;
    }

    pub(crate) fn group_snapshots(&self) -> Vec<GroupSnapshot> {
        self.group_snapshots
            .read()
            .expect("observed group snapshots poisoned")
            .clone()
    }

    pub(crate) fn with_groups<R>(&self, read: impl FnOnce(GroupProjection) -> R) -> R {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_group_delivery
            .read()
            .expect("test group delivery poisoned")
            .as_ref()
        {
            return read(GroupProjection::from_test_delivery(delivery));
        }
        let snapshots = self.group_snapshots();
        read(GroupProjection::new(&snapshots))
    }

    #[cfg(test)]
    pub(crate) fn install_test_group_delivery(&self, next: TestGroupDelivery) {
        *self
            .test_group_delivery
            .write()
            .expect("test group delivery poisoned") = Some(next);
    }

    #[cfg(test)]
    pub(crate) fn install_test_relay_delivery(&self, next: TestRelayDelivery) {
        self.recent_rows.clear();
        for event in &next.events {
            self.recent_rows.add("test", event.clone());
        }
        *self
            .test_relay_delivery
            .write()
            .expect("test relay delivery poisoned") = Some(next);
    }

    #[cfg(test)]
    fn test_relay_delivery(&self) -> Option<TestRelayDelivery> {
        self.test_relay_delivery
            .read()
            .expect("test relay delivery poisoned")
            .clone()
    }

    /// The bounded recent-rows store, populated from observation frames.
    pub(crate) fn recent_rows(&self) -> Arc<RecentRows> {
        Arc::clone(&self.recent_rows)
    }

    pub(crate) fn apply_frame(
        &self,
        observation_id: &str,
        generation: u64,
        deltas: Vec<RowDelta>,
    ) -> RowTransition {
        let transition = self
            .rows
            .lock()
            .expect("observed row views poisoned")
            .apply_frame(observation_id, generation, deltas);
        for entered in &transition.entered {
            self.recent_rows.add(
                &entered.observation_id,
                relay::relay_event(&entered.row.event),
            );
        }
        for departed in &transition.removed {
            self.recent_rows
                .remove(&departed.observation_id, &departed.row.event.id.to_hex());
        }
        transition
    }

    pub(crate) fn begin_observation(&self, observation_id: &str, generation: u64) {
        self.rows
            .lock()
            .expect("observed row views poisoned")
            .begin_observation(observation_id, generation);
    }

    pub(crate) fn close_observation(&self, observation_id: &str, generation: u64) -> RowTransition {
        let transition = self
            .rows
            .lock()
            .expect("observed row views poisoned")
            .close(observation_id, generation);
        self.recent_rows.close_observation(observation_id);
        transition
    }

    #[cfg(test)]
    pub(crate) fn row(&self, id: &EventId) -> Option<Row> {
        self.rows
            .lock()
            .expect("observed row views poisoned")
            .row(id)
            .map(|row| row.row)
    }

    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<Row> {
        self.rows
            .lock()
            .expect("observed row views poisoned")
            .rows()
            .into_iter()
            .map(|row| row.row)
            .collect()
    }

    fn rows_for_kind_author(&self, kind: u16, author: &str) -> Vec<rows::ViewRow> {
        self.rows
            .lock()
            .expect("observed row views poisoned")
            .rows_for_kind_author(kind, author)
    }
}

#[cfg(test)]
mod tests;
