//! Lifecycle-owned presentation state accumulated from NMP deliveries.
//!
//! This is deliberately process-local. It owns no replacement, relay-union,
//! freshness, or persistence rule: NMP supplies those answers. Mosaico only
//! retains the latest delivered values while their observations are alive.

use std::sync::{Arc, Mutex, RwLock};

use nmp::{AcquisitionEvidence, Row, RowDelta};
use nostr::EventId;

mod groups;
pub(crate) use groups::GroupProjection;
#[cfg(test)]
pub(crate) use groups::{TestGroup, TestGroupDelivery};
mod rows;
use rows::RowViews;
mod events;
mod messages;
pub(crate) use messages::MessageProjection;
mod profile_status;
mod reactions;
pub(crate) use reactions::ReactionProjection;
mod relay;
pub(crate) use relay::observed_profile_from_row;
#[cfg(test)]
mod test_delivery;
#[cfg(test)]
pub(crate) use test_delivery::TestRelayDelivery;

#[derive(Default)]
pub(crate) struct NmpViews {
    group_observation: RwLock<Option<Arc<nmp::nip29::GroupObservation>>>,
    #[cfg(test)]
    test_group_delivery: RwLock<Option<TestGroupDelivery>>,
    #[cfg(test)]
    test_relay_delivery: RwLock<Option<TestRelayDelivery>>,
    rows: Mutex<RowViews>,
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

impl NmpViews {
    /// Register the retained NMP observation handle. This stores no group
    /// value: every read below calls `GroupObservation::latest()`.
    pub(crate) fn set_group_observation(
        &self,
        observation: Option<Arc<nmp::nip29::GroupObservation>>,
    ) {
        *self
            .group_observation
            .write()
            .expect("NMP group observation slot poisoned") = observation;
    }

    pub(crate) fn group_snapshots(&self) -> Vec<nmp::nip29::GroupSnapshot> {
        self.group_observation
            .read()
            .expect("NMP group observation slot poisoned")
            .as_ref()
            .map(|observation| observation.latest())
            .unwrap_or_default()
    }

    pub(crate) fn with_groups<R>(&self, read: impl FnOnce(GroupProjection) -> R) -> R {
        #[cfg(test)]
        if let Some(delivery) = self
            .test_group_delivery
            .read()
            .expect("test NMP group delivery poisoned")
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
            .expect("test NMP group delivery poisoned") = Some(next);
    }

    #[cfg(test)]
    pub(crate) fn install_test_relay_delivery(&self, next: TestRelayDelivery) {
        *self
            .test_relay_delivery
            .write()
            .expect("test NMP relay delivery poisoned") = Some(next);
    }

    #[cfg(test)]
    fn test_relay_delivery(&self) -> Option<TestRelayDelivery> {
        self.test_relay_delivery
            .read()
            .expect("test NMP relay delivery poisoned")
            .clone()
    }

    pub(crate) fn apply_frame(
        &self,
        observation_id: &str,
        generation: u64,
        deltas: Vec<RowDelta>,
        evidence: Vec<AcquisitionEvidence>,
    ) -> RowTransition {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .apply_frame(observation_id, generation, deltas, evidence)
    }

    pub(crate) fn begin_observation(&self, observation_id: &str, generation: u64) {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .begin_observation(observation_id, generation);
    }

    pub(crate) fn close_observation(&self, observation_id: &str, generation: u64) -> RowTransition {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .close(observation_id, generation)
    }

    pub(crate) fn row(&self, id: &EventId) -> Option<Row> {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .row(id)
            .map(|row| row.row)
    }

    #[cfg(test)]
    pub(crate) fn rows(&self) -> Vec<Row> {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows()
            .into_iter()
            .map(|row| row.row)
            .collect()
    }

    fn rows_for_kind(&self, kind: u16) -> Vec<rows::ViewRow> {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows_for_kind(kind)
    }

    fn rows_for_kind_author(&self, kind: u16, author: &str) -> Vec<rows::ViewRow> {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows_for_kind_author(kind, author)
    }

    fn rows_for_kind_channel(&self, kind: u16, channel: &str) -> Vec<rows::ViewRow> {
        self.rows
            .lock()
            .expect("NMP row views poisoned")
            .rows_for_kind_channel(kind, channel)
    }
}

#[cfg(test)]
mod tests;
