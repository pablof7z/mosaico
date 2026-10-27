use super::super::DaemonState;
use std::sync::Arc;

/// Publish this backend's host profile once the relay path is up.
///
/// Nothing is fetched first. The profile states which workspaces this backend
/// manages, and that answer comes from the relay-signed admin lists the
/// current decoded group view exposes directly. The observation triggers a
/// fresh profile publication whenever that current answer changes.
pub(super) async fn publish_startup_profile(state: &Arc<DaemonState>) {
    state.schedule_backend_profile_refresh();
}
