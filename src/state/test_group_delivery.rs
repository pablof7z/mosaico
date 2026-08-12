use super::Store;

pub(crate) use crate::nmp_views::{TestGroup, TestGroupDelivery, TestRelayDelivery};

impl Store {
    /// Install one complete, already-aggregated NMP group delivery for a unit
    /// test. The installation replaces the prior delivery atomically and never
    /// writes SQLite.
    pub(crate) fn install_test_nmp_group_delivery(&self, delivery: TestGroupDelivery) {
        self.nmp_views.install_test_group_delivery(delivery);
    }

    /// Install one complete NMP row delivery for a unit test. The installation
    /// replaces the earlier delivery atomically and never writes SQLite.
    pub(crate) fn install_test_nmp_relay_delivery(&self, delivery: TestRelayDelivery) {
        for event_id in delivery.event_ids() {
            self.record_nmp_arrival(event_id)
                .expect("recording test NMP arrival");
        }
        self.nmp_views.install_test_relay_delivery(delivery);
    }
}
