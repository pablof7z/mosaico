//! Read-only inventory compiled only into the disposable stress binary.

use super::FavaHost;

impl FavaHost {
    /// Product-managed content observations for the standalone-daemon trial.
    /// Retained profile-feed ownership is reported by its owning component.
    pub(crate) fn stress_managed_observations(&self) -> usize {
        self.subscriptions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }
}
