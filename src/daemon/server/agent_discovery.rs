use super::DaemonState;
use crate::agent_catalog::{AgentCatalog, DiscoveryRoots, NativeAgentProfile};
use crate::session::Harness;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const REFRESH_INTERVAL: Duration = Duration::from_secs(3);

pub(crate) struct CatalogChange {
    pub(crate) removed_slugs: Vec<String>,
}

impl DaemonState {
    pub(crate) fn agent_catalog(&self) -> AgentCatalog {
        self.agent_catalog.lock().unwrap().clone()
    }

    pub(crate) fn refresh_agent_catalog(&self) -> Result<Option<CatalogChange>> {
        let roots = DiscoveryRoots::installed()?;
        let workspaces = self.with_store(|store| {
            store
                .list_workspace_bindings()
                .unwrap_or_default()
                .into_iter()
                .map(|binding| PathBuf::from(binding.abs_path))
                .collect::<Vec<_>>()
        });
        let discovered = discover(&roots, workspaces)?;
        let mut current = self.agent_catalog.lock().unwrap();
        if *current == discovered {
            return Ok(None);
        }
        let new_slugs = discovered.slugs();
        let removed_slugs = current
            .slugs()
            .into_iter()
            .filter(|slug| !new_slugs.contains(slug))
            .collect();
        *current = discovered;
        Ok(Some(CatalogChange { removed_slugs }))
    }

    pub(crate) fn resolve_native_agent(
        &self,
        slug: &str,
        workspace: Option<&Path>,
        harness: Option<Harness>,
    ) -> Result<NativeAgentProfile> {
        self.agent_catalog
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

pub(super) fn spawn_monitor(state: Arc<DaemonState>) {
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
}
