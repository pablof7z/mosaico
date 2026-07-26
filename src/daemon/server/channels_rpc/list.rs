use std::collections::BTreeSet;

use super::*;

mod projection;

#[derive(Debug, serde::Serialize)]
pub(crate) struct ChannelList {
    pub sections: Vec<ChannelListSection>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ChannelListSection {
    pub kind: &'static str,
    pub title: &'static str,
    pub channels: Vec<ChannelListEntry>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct ChannelListEntry {
    pub path: String,
    pub about: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subchannels: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<ChannelListEntry>,
}

pub(in crate::daemon::server) fn rpc_channel_list(
    state: &Arc<DaemonState>,
    params: &serde_json::Value,
) -> Result<serde_json::Value> {
    #[derive(Default, serde::Deserialize)]
    struct P {
        #[serde(default)]
        all: bool,
        #[serde(default)]
        recursive: bool,
        #[serde(default)]
        workspace: Option<String>,
    }
    let p: P = serde_json::from_value(params.clone()).context("channel_list params")?;
    let selected =
        usize::from(p.all) + usize::from(p.recursive) + usize::from(p.workspace.is_some());
    anyhow::ensure!(
        selected <= 1,
        "channel list accepts only one of all, recursive, or workspace"
    );

    let mode = if p.recursive {
        projection::ListMode::Recursive
    } else if p.all {
        projection::ListMode::All
    } else if let Some(workspace) = p.workspace {
        projection::ListMode::Workspace(normalize_workspace(&workspace)?)
    } else {
        let caller = resolve_session(state, &CallerAnchor::from_params(params))
            .context("channel list must be run from a mosaico session or with a list flag")?;
        let joined = state.with_store(|store| joined_roots(store, &caller.pubkey))?;
        projection::ListMode::Caller {
            own: caller.work_root,
            joined,
        }
    };

    let backend = state.backend_pubkey().unwrap_or_default();
    let list = state
        .with_store(|store| projection::build(store, mode, crate::util::now_secs(), &backend))?;
    Ok(serde_json::to_value(list)?)
}

fn normalize_workspace(value: &str) -> Result<String> {
    let workspace = value.trim().trim_start_matches('/');
    anyhow::ensure!(!workspace.is_empty(), "workspace must not be empty");
    anyhow::ensure!(
        !workspace.contains('/'),
        "workspace must be a root name, not a channel path"
    );
    Ok(workspace.to_string())
}

fn joined_roots(store: &crate::state::Store, pubkey: &str) -> Result<BTreeSet<String>> {
    let mut roots = BTreeSet::new();
    for (channel, _) in store.list_session_routes(pubkey)? {
        match crate::daemon::workspace_path::WorkspacePathResolver::new(store)
            .root_for_channel(&channel)
        {
            Ok(root) if !root.is_empty() => {
                roots.insert(root);
            }
            Ok(_) => {}
            Err(error) => tracing::debug!(
                channel,
                %error,
                "channel list: ignoring joined route with incomplete ancestry"
            ),
        }
    }
    Ok(roots)
}

#[cfg(test)]
mod tests;
