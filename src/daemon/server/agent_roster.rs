use super::*;

#[derive(Debug, serde::Serialize)]
pub(in crate::daemon::server) struct RosterPublishReport {
    pub(in crate::daemon::server) published: usize,
    pub(in crate::daemon::server) removed: usize,
    pub(in crate::daemon::server) failed: Vec<String>,
}

#[derive(serde::Deserialize, Default)]
pub(in crate::daemon::server) struct PublishRosterParams {
    #[serde(default)]
    remove_slug: Option<String>,
}

pub(in crate::daemon::server) async fn rpc_agent_roster_publish(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: PublishRosterParams =
        serde_json::from_value(params.clone()).context("agent_roster_publish params")?;
    let report = publish_local_agent_roster(state, params.remove_slug.as_deref()).await?;
    Ok(serde_json::to_value(report)?)
}

pub(in crate::daemon::server) async fn publish_local_agent_roster(
    state: &Arc<DaemonState>,
    remove_slug: Option<&str>,
) -> Result<RosterPublishReport> {
    let root_channels = state.with_store(root_channels);
    let edge_home = crate::config::edge_home();
    let local_agents = crate::identity::list_local_agents(&edge_home);
    let mut failed = Vec::new();
    let mut published = 0usize;
    let mut removed = 0usize;

    if let Some(slug) = remove_slug.map(str::trim).filter(|s| !s.is_empty()) {
        match state
            .provider
            .publish_agent_roster(slug, &state.host, "", &[])
            .await
        {
            Ok(_) => removed += 1,
            Err(e) => failed.push(format!("{slug}: {e:#}")),
        }
    }

    for (slug, _commands, _agent_def, byline) in local_agents {
        let use_criteria = byline.unwrap_or_default();
        match state
            .provider
            .publish_agent_roster(&slug, &state.host, &use_criteria, &root_channels)
            .await
        {
            Ok(_) => published += 1,
            Err(e) => failed.push(format!("{slug}: {e:#}")),
        }
    }

    Ok(RosterPublishReport {
        published,
        removed,
        failed,
    })
}

fn root_channels(store: &Store) -> Vec<String> {
    store
        .list_projects_read_model()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !c.is_archived())
        .map(|c| c.channel_h)
        .collect()
}
