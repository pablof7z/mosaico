use super::super::{agent_roster::publish_local_agent_roster, DaemonState};
use std::sync::Arc;

pub(super) async fn publish_startup_roster(state: &Arc<DaemonState>) {
    state.provider.refresh_project_list().await.ok();
    match publish_local_agent_roster(state, None).await {
        Ok(report) => tracing::info!(
            published = report.published,
            removed = report.removed,
            failed = report.failed.len(),
            "published backend agent roster"
        ),
        Err(e) => tracing::warn!(error = %e, "backend agent roster publish failed"),
    }
}

pub(super) fn seed_spawn_on_mention_coverage(state: &Arc<DaemonState>) {
    let member_groups: Vec<String> = state.with_store(|s| {
        let mut pubkeys = s.list_identity_pubkeys().unwrap_or_default();
        if let Some(pk) = state.backend_pubkey() {
            pubkeys.push(pk);
        }
        let mut groups = Vec::new();
        for pk in &pubkeys {
            if let Ok(gs) = s.list_channels_where_member(pk) {
                groups.extend(gs);
            }
            if let Ok(gs) = s.list_channels_where_admin(pk) {
                groups.extend(gs);
            }
        }
        groups.sort_unstable();
        groups.dedup();
        groups
    });
    {
        let mut projs = state.subscribed_projects.lock().unwrap();
        for group in &member_groups {
            if !projs.iter().any(|p| p == group) {
                projs.push(group.clone());
            }
        }
    }
    tracing::info!(
        subscribed_identity_pubkeys =
            state.with_store(|s| s.list_identity_pubkeys().unwrap_or_default().len()),
        member_groups = member_groups.len(),
        "spawn-on-mention coverage seeded"
    );
}
