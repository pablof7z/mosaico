use super::super::{agent_roster::publish_local_agent_roster, DaemonState};
use std::sync::Arc;

pub(super) async fn publish_startup_roster(state: &Arc<DaemonState>) {
    state.provider.refresh_root_channels().await.ok();
    restore_workspace_root_names(state).await;
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

async fn restore_workspace_root_names(state: &Arc<DaemonState>) {
    let Some(backend_pubkey) = state.backend_pubkey() else {
        return;
    };
    let roots = match state.with_store(|store| roots_needing_workspace_name(store, &backend_pubkey))
    {
        Ok(roots) => roots,
        Err(error) => {
            tracing::error!(%error, "workspace root repair authority lookup failed");
            return;
        }
    };
    for root in roots {
        if state.provider.nip29_set_group_name(&root, &root).await {
            state.provider.fetch_and_materialize_channel(&root).await;
        } else {
            tracing::warn!(
                channel = %root,
                "workspace root name repair was rejected"
            );
        }
    }
}

fn roots_needing_workspace_name(
    store: &crate::state::Store,
    backend_pubkey: &str,
) -> anyhow::Result<Vec<String>> {
    let mut roots = Vec::new();
    for channel in store.list_root_channels()? {
        if channel.name.trim() == channel.channel_h {
            continue;
        }
        if store.is_channel_admin(&channel.channel_h, backend_pubkey)?
            || crate::daemon::workspace_path::WorkspacePathResolver::new(store)
                .path_for_channel(&channel.channel_h)?
                .is_some()
        {
            roots.push(channel.channel_h);
        }
    }
    Ok(roots)
}

pub(super) fn seed_spawn_on_mention_coverage(state: &Arc<DaemonState>) {
    let member_groups: Vec<String> = state.with_store(|s| {
        let mut pubkeys = s.list_local_session_pubkeys().unwrap_or_default();
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
        let mut projs = state.subscriptions.roots.lock().unwrap();
        for group in &member_groups {
            if !projs.iter().any(|p| p == group) {
                projs.push(group.clone());
            }
        }
    }
    tracing::info!(
        subscribed_session_pubkeys =
            state.with_store(|s| s.list_local_session_pubkeys().unwrap_or_default().len()),
        member_groups = member_groups.len(),
        "spawn-on-mention coverage seeded"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_managed_misnamed_roots_need_repair() {
        let store = crate::state::Store::open_memory().unwrap();
        store.upsert_channel("one", "wrong", "", "", 1).unwrap();
        store.upsert_channel("two", "two", "", "", 1).unwrap();
        store.upsert_channel("remote", "remote", "", "", 1).unwrap();
        store
            .upsert_channel("bound", "also-wrong", "", "", 1)
            .unwrap();
        store.upsert_workspace("bound", "/work/bound", 1).unwrap();
        store
            .upsert_channel_member("one", "backend", "admin", 1)
            .unwrap();
        store
            .upsert_channel_member("two", "backend", "admin", 1)
            .unwrap();

        assert_eq!(
            roots_needing_workspace_name(&store, "backend").unwrap(),
            vec!["bound", "one"]
        );
    }
}
