use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::daemon::server) struct CapabilityAdvertisement {
    pub(in crate::daemon::server) slug: String,
    pub(in crate::daemon::server) use_criteria: String,
    pub(in crate::daemon::server) root_channels: Vec<String>,
    pub(in crate::daemon::server) available_since: u64,
}

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

pub(in crate::daemon::server) fn rpc_agent_roster_refresh(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    let params: PublishRosterParams =
        serde_json::from_value(params.clone()).context("agent_roster_refresh params")?;
    state.refresh_agent_catalog()?;
    reconcile_local_agent_roster(state)?;
    state.schedule_agent_roster_refresh(params.remove_slug.into_iter().collect());
    Ok(serde_json::json!({"scheduled": true}))
}

/// Synchronously prune this backend's own cached 30555 rows down to its live
/// advertised capability set. `mosaico my session` reads that cache directly
/// (`Store::list_agent_roster`), so this makes a deleted agent disappear at
/// once instead of waiting on the detached, best-effort tombstone publish in
/// `schedule_agent_roster_refresh` — which can fail to reach the relay, target
/// a stale `(backend_pubkey, agent_slug)` address, or be lost across a restart.
/// Other backends' advertisements are left untouched.
fn reconcile_local_agent_roster(state: &Arc<DaemonState>) -> Result<()> {
    let Some(backend_pubkey) = state.backend_pubkey() else {
        return Ok(());
    };
    let (advertisements, _failed) = capability_advertisements(state)?;
    let keep = advertisements
        .into_iter()
        .map(|advertisement| advertisement.slug)
        .collect::<Vec<_>>();
    let removed =
        state.with_store(|store| store.retain_local_agent_roster(&backend_pubkey, &keep))?;
    if removed > 0 {
        tracing::debug!(backend = %backend_pubkey, removed, "pruned stale local agent roster rows");
    }
    Ok(())
}

pub(in crate::daemon::server) async fn publish_local_agent_roster(
    state: &Arc<DaemonState>,
    remove_slug: Option<&str>,
) -> Result<RosterPublishReport> {
    let (advertisements, mut failed) = capability_advertisements(state)?;
    let mut published = 0usize;
    let mut removed = 0usize;

    if let Some(slug) = remove_slug
        .map(str::trim)
        .filter(|slug| should_tombstone(&advertisements, slug))
    {
        match state
            .provider
            .publish_agent_roster(slug, &state.host, "", &[])
            .await
        {
            Ok(_) => removed += 1,
            Err(e) => failed.push(format!("{slug}: {e:#}")),
        }
    }

    for advertisement in advertisements {
        match state
            .provider
            .publish_agent_roster(
                &advertisement.slug,
                &state.host,
                &advertisement.use_criteria,
                &advertisement.root_channels,
            )
            .await
        {
            Ok(_) => published += 1,
            Err(e) => failed.push(format!("{}: {e:#}", advertisement.slug)),
        }
    }

    // Re-publish the backend kind:0 so its advertised `agent` tags track the
    // managed set — clients (e.g. the 29er add-agent picker) see add/remove
    // changes without a daemon restart, mirroring this roster republish.
    publish_backend_profile(state).await;

    Ok(RosterPublishReport {
        published,
        removed,
        failed,
    })
}

fn should_tombstone(advertisements: &[CapabilityAdvertisement], slug: &str) -> bool {
    !slug.is_empty()
        && !advertisements
            .iter()
            .any(|advertisement| advertisement.slug == slug)
}

impl DaemonState {
    pub(crate) fn schedule_agent_roster_refresh(self: &Arc<Self>, removed_slugs: Vec<String>) {
        let state = self.clone();
        tokio::spawn(async move {
            for slug in removed_slugs {
                if let Err(error) = publish_local_agent_roster(&state, Some(&slug)).await {
                    tracing::warn!(slug, error = %format!("{error:#}"), "selected agent combination retirement failed");
                }
            }
            if let Err(error) = publish_local_agent_roster(&state, None).await {
                tracing::warn!(error = %format!("{error:#}"), "selected agent binding roster publish failed");
            }
        });
    }
}

/// Publish the backend process's own kind:0 identity, advertising the managed
/// agents as `["agent", slug, description]` tags. Best-effort: a failure is
/// logged and deferred to the next trigger (startup or the next roster change).
/// Called from daemon startup and whenever the managed-agent set changes.
pub(in crate::daemon::server) async fn publish_backend_profile(state: &Arc<DaemonState>) {
    let Some(backend_keys) = state.provider.management_keys() else {
        return;
    };
    let (advertisements, failures) = match capability_advertisements(state) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "agent capability authority lookup failed");
            return;
        }
    };
    for failure in failures {
        tracing::error!(error = %failure, "agent capability is not advertisable");
    }
    let agents = advertisements
        .into_iter()
        .map(|agent| (agent.slug, agent.use_criteria))
        .collect();
    let profile = crate::domain::Profile::backend_named(
        backend_keys.public_key().to_hex(),
        format!("{} (mosaico)", state.host),
        state.host.clone(),
        state.owners.clone(),
    )
    .with_agents(agents);
    let ev = crate::domain::DomainEvent::Profile(profile);
    if let Err(e) = state.provider.enqueue(&ev, &backend_keys).await {
        tracing::warn!(error = %e, "backend kind:0 profile publish failed");
    }
}

fn root_channels(store: &Store) -> Result<Vec<String>> {
    Ok(store
        .list_root_channels()?
        .into_iter()
        .filter(|c| !c.is_archived())
        .map(|c| c.channel_h)
        .collect())
}

pub(in crate::daemon::server) fn capability_advertisements(
    state: &Arc<DaemonState>,
) -> Result<(Vec<CapabilityAdvertisement>, Vec<String>)> {
    let roots = state.with_store(root_channels)?;
    let root_set = roots.iter().cloned().collect::<BTreeSet<_>>();
    let bindings = state.with_store(|store| {
        crate::daemon::workspace_path::WorkspacePathResolver::new(store).bindings()
    })?;
    let mut merged = BTreeMap::<String, (String, u64, BTreeSet<String>)>::new();
    let catalog = state.agent_catalog();
    let installed_harnesses = state.installed_harnesses();
    let mut failed = Vec::new();
    let harnesses = match crate::harness::HarnessesConfig::load() {
        Ok(config) => config,
        Err(error) => {
            failed.push(format!("harnesses.json: {error:#}"));
            crate::harness::HarnessesConfig::default()
        }
    };
    merge_inventory(
        &mut merged,
        crate::agent_inventory::AgentInventory::build(
            &crate::config::mosaico_home(),
            &installed_harnesses,
            &harnesses,
            &catalog,
            None,
        ),
        &root_set,
        &mut failed,
    );
    for binding in bindings {
        if !root_set.contains(&binding.channel_h) {
            continue;
        }
        merge_inventory(
            &mut merged,
            crate::agent_inventory::AgentInventory::build(
                &crate::config::mosaico_home(),
                &installed_harnesses,
                &harnesses,
                &catalog,
                Some(std::path::Path::new(&binding.abs_path)),
            ),
            &BTreeSet::from([binding.channel_h]),
            &mut failed,
        );
    }

    let advertisements = merged
        .into_iter()
        .map(
            |(slug, (use_criteria, available_since, root_channels))| CapabilityAdvertisement {
                slug,
                use_criteria,
                root_channels: root_channels.into_iter().collect(),
                available_since,
            },
        )
        .collect();
    failed.sort();
    failed.dedup();
    Ok((advertisements, failed))
}

fn merge_inventory(
    merged: &mut BTreeMap<String, (String, u64, BTreeSet<String>)>,
    inventory: crate::agent_inventory::AgentInventory,
    roots: &BTreeSet<String>,
    failed: &mut Vec<String>,
) {
    failed.extend(inventory.failures);
    for agent in inventory.agents {
        let entry = merged
            .entry(agent.slug)
            .or_insert_with(|| (agent.use_criteria, agent.available_since, BTreeSet::new()));
        entry.1 = entry.1.min(agent.available_since);
        entry.2.extend(roots.iter().cloned());
    }
}

#[cfg(test)]
#[path = "agent_roster/tests.rs"]
mod tests;
