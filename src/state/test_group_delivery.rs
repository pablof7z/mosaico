use super::Store;

pub(crate) use crate::fava_views::{TestGroup, TestGroupDelivery, TestRelayDelivery};

impl Store {
    /// Install one complete, already-aggregated Fava group delivery for a unit
    /// test. The installation replaces the prior delivery atomically and never
    /// writes SQLite.
    pub(crate) fn install_test_fava_group_delivery(&self, delivery: TestGroupDelivery) {
        self.fava_views.install_test_group_delivery(delivery);
    }

    /// Install one complete Fava row delivery for a unit test. The installation
    /// replaces the earlier delivery atomically and never writes SQLite.
    pub(crate) fn install_test_fava_relay_delivery(&self, delivery: TestRelayDelivery) {
        for event_id in delivery.event_ids() {
            self.record_fava_arrival(event_id)
                .expect("recording test Fava arrival");
        }
        let profiles = delivery.profiles_for_feed();
        self.fava_views.install_test_relay_delivery(delivery);
        // `Store::get_profile` reads from the retained profile feed; route the
        // synthetic profiles to the feed's test seam so Store-logic tests that
        // need profile data (without a real Fava engine) keep working.
        self.profile_feed.install_test_profiles(profiles);
    }
}
