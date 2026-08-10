//! Read-only inventory compiled only into the disposable stress binary.

use super::NmpHost;

impl NmpHost {
    /// Exact product-observation inventory for the standalone-daemon trial.
    /// Production builds expose no diagnostic RPC or alternate ownership door.
    pub(crate) fn stress_snapshot(&self) -> serde_json::Value {
        let subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let profile_observations = subscriptions
            .keys()
            .filter(|id| id.starts_with("mosaico-profile-"))
            .count();
        serde_json::json!({
            "managed_observations": subscriptions.len(),
            "profile_observations": profile_observations,
        })
    }
}
