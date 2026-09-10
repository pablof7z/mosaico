use super::{Profile, RelayEvent, Status, Store};

pub(crate) use crate::group_projection::{TestGroup, TestGroupDelivery};

#[derive(Clone, Default)]
pub(crate) struct TestRelayDelivery {
    pub(super) profiles: Vec<Profile>,
    pub(super) statuses: Vec<Status>,
    pub(super) events: Vec<RelayEvent>,
}

#[derive(Default)]
pub(super) struct TestSnapshots {
    pub(super) groups: Option<TestGroupDelivery>,
    pub(super) relay: Option<TestRelayDelivery>,
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
        self.test_snapshots
            .write()
            .expect("test snapshots poisoned")
            .groups = Some(delivery);
    }

    pub(crate) fn install_test_relay_delivery(&self, delivery: TestRelayDelivery) {
        for event_id in delivery.event_ids() {
            self.record_event_arrival(event_id)
                .expect("recording test event arrival");
        }
        self.test_snapshots
            .write()
            .expect("test snapshots poisoned")
            .relay = Some(delivery);
    }
}
