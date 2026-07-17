use super::DaemonState;
use crate::agent_catalog::{AgentCatalog, DiscoveryRoots, NativeAgentProfile};
use crate::session::Harness;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

pub(crate) struct CatalogChange {
    pub(crate) removed_slugs: Vec<String>,
}

pub(in crate::daemon::server) fn rpc_agent_inventory(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(serde::Deserialize)]
    struct Params {
        #[serde(default)]
        cwd: Option<PathBuf>,
    }

    let params: Params =
        serde_json::from_value(params.clone()).context("agent_inventory params")?;
    let inventory = state.agent_inventory(params.cwd.as_deref())?;
    Ok(serde_json::to_value(inventory)?)
}

impl DaemonState {
    pub(crate) fn agent_inventory(
        &self,
        workspace: Option<&Path>,
    ) -> Result<crate::agent_inventory::AgentInventory> {
        self.refresh_agent_catalog()?;
        let harnesses = crate::harness::HarnessesConfig::load()?;
        Ok(crate::agent_inventory::AgentInventory::build(
            &crate::config::mosaico_home(),
            &self.installed_harnesses(),
            &harnesses,
            &self.agent_catalog(),
            workspace,
        ))
    }

    pub(crate) fn installed_harnesses(&self) -> Vec<Harness> {
        self.catalog.harnesses.lock().unwrap().clone()
    }

    pub(crate) fn agent_catalog(&self) -> AgentCatalog {
        self.catalog.agents.lock().unwrap().clone()
    }

    pub(crate) fn refresh_agent_catalog(&self) -> Result<Option<CatalogChange>> {
        let roots = DiscoveryRoots::installed()?;
        let workspaces = self.with_store(|store| -> Result<Vec<PathBuf>> {
            let bindings =
                crate::daemon::workspace_path::WorkspacePathResolver::new(store).bindings()?;
            Ok(bindings
                .into_iter()
                .map(|binding| PathBuf::from(binding.abs_path))
                .collect())
        })?;
        let discovered = discover(&roots, workspaces)?;
        let installed = crate::config::detect_available_harnesses()?;
        let current = self.catalog.agents.lock().unwrap().clone();
        let current_harnesses = self.installed_harnesses();
        if current == discovered && current_harnesses == installed {
            return Ok(None);
        }
        let new_slugs = discovered.slugs();
        let mut removed_slugs = current
            .slugs()
            .into_iter()
            .filter(|slug| !new_slugs.contains(slug))
            .collect::<Vec<_>>();
        removed_slugs.extend(
            current_harnesses
                .iter()
                .filter(|harness| !installed.contains(harness))
                .map(|harness| harness.agent_slug().to_string()),
        );
        removed_slugs.sort();
        removed_slugs.dedup();
        *self.catalog.agents.lock().unwrap() = discovered;
        *self.catalog.harnesses.lock().unwrap() = installed;
        Ok(Some(CatalogChange { removed_slugs }))
    }

    pub(crate) fn resolve_native_agent(
        &self,
        slug: &str,
        workspace: Option<&Path>,
        harness: Option<Harness>,
    ) -> Result<NativeAgentProfile> {
        self.catalog
            .agents
            .lock()
            .unwrap()
            .resolve(slug, workspace, harness)
    }
}

fn discover(roots: &DiscoveryRoots, mut workspaces: Vec<PathBuf>) -> Result<AgentCatalog> {
    workspaces.sort();
    workspaces.dedup();
    AgentCatalog::discover(roots, &workspaces)
}

pub(super) fn start_monitor(state: Arc<DaemonState>) {
    if let Err(error) = state.refresh_agent_catalog() {
        tracing::warn!(error = %format!("{error:#}"), "native agent discovery failed");
    }
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            match state.refresh_agent_catalog() {
                Ok(None) => {}
                Ok(Some(change)) => {
                    tracing::info!("native agent catalog changed");
                    for slug in &change.removed_slugs {
                        if let Err(error) =
                            super::agent_roster::publish_local_agent_roster(&state, Some(slug))
                                .await
                        {
                            tracing::warn!(
                                slug,
                                error = %format!("{error:#}"),
                                "removed native agent roster publish failed"
                            );
                        }
                    }
                    if let Err(error) =
                        super::agent_roster::publish_local_agent_roster(&state, None).await
                    {
                        tracing::warn!(
                            error = %format!("{error:#}"),
                            "native agent catalog roster publish failed"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    error = %format!("{error:#}"),
                    "native agent catalog refresh rejected"
                ),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env::EnvGuard;
    use tempfile::TempDir;

    #[test]
    fn discovery_deduplicates_workspace_bindings() {
        let home = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let path = workspace.path().join(".codex/agents/reviewer.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            "name='reviewer'\ndescription='review'\ndeveloper_instructions='review'",
        )
        .unwrap();
        let catalog = discover(
            &DiscoveryRoots::for_user_home(home.path()),
            vec![
                workspace.path().to_path_buf(),
                workspace.path().to_path_buf(),
            ],
        )
        .unwrap();
        assert_eq!(catalog.capabilities(Some(workspace.path())).len(), 1);
    }

    #[tokio::test]
    async fn rpc_serves_durable_inventory_without_a_cli_keystore_read() {
        let root = TempDir::new().unwrap();
        let mosaico_home = root.path().join(".mosaico");
        std::fs::create_dir_all(&mosaico_home).unwrap();
        std::fs::write(
            mosaico_home.join("harnesses.json"),
            r#"{"codex-pty":{"harness":"codex","transport":"pty"}}"#,
        )
        .unwrap();
        let mut env = EnvGuard::set("HOME", root.path());
        env.set_var("MOSAICO_HOME", &mosaico_home);
        env.set_var("MOSAICO_ISOLATED_HOME_OK", "1");
        crate::identity::add_local_agent(&mosaico_home, "writer", "codex-pty", None, 7).unwrap();
        let state = DaemonState::new_for_test().await;

        let value =
            rpc_agent_inventory(&state, &serde_json::json!({ "cwd": root.path() })).unwrap();
        let inventory: crate::agent_inventory::AgentInventory =
            serde_json::from_value(value).unwrap();
        let writer = inventory.find("writer").expect("durable writer");
        assert!(matches!(
            writer.source,
            crate::agent_inventory::AgentSource::Durable { .. }
        ));
    }
}
