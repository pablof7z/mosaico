use super::super::{backend_profile::publish_backend_profile, DaemonState};
use std::sync::Arc;

/// Publish this backend's host profile once the relay path is up.
///
/// Nothing is fetched first. The profile states which workspaces this backend
/// manages, and that answer comes from the relay-signed admin lists the
/// retained NMP group observation exposes directly. The observation triggers a
/// fresh profile publication whenever that current answer changes.
pub(super) async fn publish_startup_profile(state: &Arc<DaemonState>) {
    match publish_backend_profile(state).await {
        Ok(report) => tracing::info!(
            agents = report.agents,
            workspaces = report.workspaces,
            failed = report.failed.len(),
            "published backend host profile"
        ),
        Err(e) => tracing::warn!(error = %e, "backend host profile publish failed"),
    }
}
