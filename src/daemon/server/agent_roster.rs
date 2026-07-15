use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
struct CapabilityAdvertisement {
    slug: String,
    use_criteria: String,
    root_channels: Vec<String>,
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
    let (advertisements, mut failed) = capability_advertisements(state);
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

/// Publish the backend process's own kind:0 identity, advertising the managed
/// agents as `["agent", slug, description]` tags. Best-effort: a failure is
/// logged and deferred to the next trigger (startup or the next roster change).
/// Called from daemon startup and whenever the managed-agent set changes.
pub(in crate::daemon::server) async fn publish_backend_profile(state: &Arc<DaemonState>) {
    let Some(backend_keys) = state.provider.management_keys() else {
        return;
    };
    let (advertisements, failures) = capability_advertisements(state);
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
    if let Err(e) = state.provider.publish(&ev, &backend_keys).await {
        tracing::warn!(error = %e, "backend kind:0 profile publish failed");
    }
}

fn root_channels(store: &Store) -> Vec<String> {
    store
        .list_root_channels()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !c.is_archived())
        .map(|c| c.channel_h)
        .collect()
}

fn capability_advertisements(
    state: &Arc<DaemonState>,
) -> (Vec<CapabilityAdvertisement>, Vec<String>) {
    let roots = state.with_store(root_channels);
    let root_set = roots.iter().cloned().collect::<BTreeSet<_>>();
    let bindings = state.with_store(|store| store.list_workspace_bindings().unwrap_or_default());
    let local_agents = crate::identity::list_local_agents(&crate::config::mosaico_home());
    let configured = local_agents
        .iter()
        .map(|(slug, _, _, _)| slug.clone())
        .collect::<BTreeSet<_>>();
    let mut merged = BTreeMap::<String, (String, BTreeSet<String>)>::new();
    for (slug, _, _, byline) in local_agents {
        merged.insert(slug, (byline.unwrap_or_default(), root_set.clone()));
    }

    let catalog = state.agent_catalog();
    let mut failed = Vec::new();
    let harnesses = match crate::harness::HarnessesConfig::load() {
        Ok(config) => config,
        Err(error) => {
            failed.push(format!("harnesses.json: {error:#}"));
            crate::harness::HarnessesConfig::default()
        }
    };
    merge_discovered(
        &mut merged,
        &configured,
        catalog.capabilities(None),
        &root_set,
        &harnesses,
        &mut failed,
    );
    for binding in bindings {
        if !root_set.contains(&binding.channel_h) {
            continue;
        }
        merge_discovered(
            &mut merged,
            &configured,
            catalog.capabilities(Some(std::path::Path::new(&binding.abs_path))),
            &BTreeSet::from([binding.channel_h]),
            &harnesses,
            &mut failed,
        );
    }

    let advertisements = merged
        .into_iter()
        .map(
            |(slug, (use_criteria, root_channels))| CapabilityAdvertisement {
                slug,
                use_criteria,
                root_channels: root_channels.into_iter().collect(),
            },
        )
        .collect();
    failed.sort();
    failed.dedup();
    (advertisements, failed)
}

fn merge_discovered(
    merged: &mut BTreeMap<String, (String, BTreeSet<String>)>,
    configured: &BTreeSet<String>,
    capabilities: Vec<crate::agent_catalog::AgentCapability>,
    roots: &BTreeSet<String>,
    harnesses: &crate::harness::HarnessesConfig,
    failed: &mut Vec<String>,
) {
    for capability in capabilities {
        if configured.contains(&capability.slug) {
            continue;
        }
        if capability.profiles.len() != 1 {
            let harnesses = capability
                .profiles
                .iter()
                .map(|profile| profile.harness.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            failed.push(format!(
                "{}: installed by multiple harnesses ({harnesses}); add an explicit agent harness binding",
                capability.slug
            ));
            continue;
        }
        let harness = capability.profiles[0].harness;
        if let Err(error) = crate::harness::native_bundle_with(harnesses, harness) {
            failed.push(format!("{}: {error:#}", capability.slug));
            continue;
        }
        let entry = merged
            .entry(capability.slug)
            .or_insert_with(|| (capability.use_criteria, BTreeSet::new()));
        entry.1.extend(roots.iter().cloned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;

    fn write(path: &std::path::Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[tokio::test]
    async fn discovered_capabilities_are_global_or_workspace_scoped() {
        let home = tempfile::tempdir().unwrap();
        let mosaico_home = home.path().join("mosaico");
        let codex_home = home.path().join(".codex");
        let mut env = EnvGuard::set("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        env.set_var("HOME", home.path());
        env.set_var("CODEX_HOME", &codex_home);
        write(
            &mosaico_home.join("harnesses.json"),
            r#"{"codex-rpc":{"harness":"codex","transport":"app-server"}}"#,
        );
        write(
            &codex_home.join("agents/global.toml"),
            "name='global'\ndescription='Everywhere'\ndeveloper_instructions='Work'",
        );
        let work_a = home.path().join("work-a");
        let work_b = home.path().join("work-b");
        std::fs::create_dir_all(&work_b).unwrap();
        write(
            &work_a.join(".codex/agents/project.toml"),
            "name='project'\ndescription='Only A'\ndeveloper_instructions='Work'",
        );
        let state = DaemonState::new_for_test().await;
        state.with_store(|store| {
            store.upsert_channel("root-a", "root-a", "", "", 1).unwrap();
            store.upsert_channel("root-b", "root-b", "", "", 1).unwrap();
            store
                .upsert_workspace("root-a", &work_a.to_string_lossy(), 1)
                .unwrap();
            store
                .upsert_workspace("root-b", &work_b.to_string_lossy(), 1)
                .unwrap();
        });
        state.refresh_agent_catalog().unwrap();

        let (advertised, failed) = capability_advertisements(&state);
        assert!(failed.is_empty(), "{failed:?}");
        let global = advertised
            .iter()
            .find(|agent| agent.slug == "global")
            .unwrap();
        assert_eq!(global.root_channels, ["root-a", "root-b"]);
        let project = advertised
            .iter()
            .find(|agent| agent.slug == "project")
            .unwrap();
        assert_eq!(project.root_channels, ["root-a"]);
    }
}
