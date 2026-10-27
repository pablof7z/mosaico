use std::sync::Arc;

use super::{Profile, RelayEvent, Status, Store};
use crate::fabric::nip29::view::Nip29View;

pub(crate) use crate::group_projection::{TestGroup, TestGroupDelivery};

#[derive(Clone, Default)]
pub(crate) struct TestRelayDelivery {
    profiles: Vec<Profile>,
    statuses: Vec<Status>,
    events: Vec<RelayEvent>,
}

/// Installed deliveries replace the whole real view: once either is
/// installed, every read comes from the level assembled from both.
#[derive(Default)]
pub(super) struct TestSnapshots {
    groups: Option<TestGroupDelivery>,
    relay: Option<TestRelayDelivery>,
    /// The level assembled from both deliveries, rebuilt on each install.
    pub(super) view: Option<Arc<Nip29View>>,
}

impl TestSnapshots {
    fn assemble(&mut self) {
        let relay = self.relay.clone().unwrap_or_default();
        self.view = Some(Arc::new(Nip29View::from_parts(
            self.groups
                .as_ref()
                .map(TestGroupDelivery::projection)
                .unwrap_or_default(),
            relay.profiles,
            relay.statuses,
            relay.events,
        )));
    }
}

impl TestRelayDelivery {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn profiles(mut self, profiles: impl IntoIterator<Item = Profile>) -> Self {
        self.profiles = profiles.into_iter().collect();
        self
    }

    pub(crate) fn statuses(mut self, statuses: impl IntoIterator<Item = Status>) -> Self {
        self.statuses = statuses.into_iter().collect();
        self
    }

    pub(crate) fn events(mut self, events: impl IntoIterator<Item = RelayEvent>) -> Self {
        self.events = events.into_iter().collect();
        self
    }

    pub(crate) fn event_ids(&self) -> impl Iterator<Item = &str> {
        self.events.iter().map(|event| event.id.as_str())
    }
}

impl Store {
    pub(crate) fn install_test_group_delivery(&self, delivery: TestGroupDelivery) {
        let mut snapshots = self
            .test_snapshots
            .write()
            .expect("test snapshots poisoned");
        snapshots.groups = Some(delivery);
        snapshots.assemble();
    }

    pub(crate) fn install_test_relay_delivery(&self, delivery: TestRelayDelivery) {
        for event_id in delivery.event_ids() {
            self.record_event_arrival(event_id)
                .expect("recording test event arrival");
        }
        let mut snapshots = self
            .test_snapshots
            .write()
            .expect("test snapshots poisoned");
        snapshots.relay = Some(delivery);
        snapshots.assemble();
    }
}
