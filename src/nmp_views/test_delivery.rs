use crate::state::{Profile, RelayEvent, Status};

/// One complete NMP row delivery for deterministic consumer tests.
/// Installing a later value replaces the whole earlier delivery.
#[derive(Clone, Default)]
pub(crate) struct TestRelayDelivery {
    pub(super) profiles: Vec<Profile>,
    pub(super) statuses: Vec<Status>,
    pub(super) events: Vec<RelayEvent>,
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

    /// The profiles this delivery carries, for routing to the profile feed's
    /// test seam when `Store::get_profile` reads from the feed.
    pub(crate) fn profiles_for_feed(&self) -> Vec<Profile> {
        self.profiles.clone()
    }
}
